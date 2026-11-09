use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use url::Url;

use super::requirement_course_codes::CourseCodeParser;
use super::requirement_manifest::{
    exclude_unhashed_primary_manifest_duplicates, preselect_parseable_manifest_entries,
};
use super::requirement_plan_matching::{
    build_primary_major_titles_by_plan_id, filter_manifest_entries_for_specialized_plan_title,
};

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourcePipelineValidationCheck {
    pub id: String,
    pub label: String,
    pub status: String,
    pub details: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourcePipelineValidationMetrics {
    pub validation_target_plan_id: Option<String>,
    pub eligible_auto_promotion_owner_count: usize,
    pub eligible_auto_promotion_owner_ids: Vec<String>,
    pub eligible_auto_promotion_owner_keys: Vec<String>,
    pub weak_existing_owner_count: usize,
    pub high_confidence_replacement_owner_count: usize,
    pub review_queue_owner_count: usize,
    pub source_gap_owner_count: usize,
    pub promoted_owner_count: usize,
    pub promoted_owner_keys: Vec<String>,
    pub canonical_primary_owner_count: usize,
    pub parseable_primary_owner_count: usize,
    pub parsed_owner_count: usize,
    pub requirement_fingerprint_owner_count: usize,
}

pub struct SourcePipelineValidationMetricInputs<'a> {
    pub validation_target_plan_id: Option<&'a str>,
    pub eligible_auto_promotion_owners: &'a [Value],
    pub discovery_report: &'a Value,
    pub review_owner_keys: &'a HashSet<String>,
    pub source_gap_owner_keys: &'a HashSet<String>,
    pub promoted_owner_ids: &'a HashSet<String>,
    pub promoted_owner_keys: &'a HashSet<String>,
    pub primary_manifest_owner_ids: &'a HashSet<String>,
    pub parseable_primary_owner_ids: &'a HashSet<String>,
    pub parsed_owner_ids: &'a HashSet<String>,
    pub requirement_fingerprint_owner_ids: &'a HashSet<String>,
}

pub fn build_source_pipeline_validation_metrics(
    inputs: SourcePipelineValidationMetricInputs<'_>,
) -> SourcePipelineValidationMetrics {
    let eligible_owner_ids = inputs
        .eligible_auto_promotion_owners
        .iter()
        .map(|owner| js_property_string(owner.get("ownerId")))
        .collect::<HashSet<_>>();
    let eligible_owner_keys = inputs
        .eligible_auto_promotion_owners
        .iter()
        .map(|owner| js_property_string(owner.get("ownerKey")))
        .collect::<HashSet<_>>();
    let weak_existing_owners = value_array(inputs.discovery_report.get("weakExistingOwners"));
    let high_confidence_replacement_owner_count = inputs
        .discovery_report
        .get("highConfidenceReplacementCount")
        .filter(|value| !value.is_null())
        .and_then(Value::as_u64)
        .and_then(|count| usize::try_from(count).ok())
        .unwrap_or_else(|| {
            weak_existing_owners
                .iter()
                .filter(|owner| {
                    owner.get("suggestedAction").and_then(Value::as_str)
                        == Some("replace-existing-primary")
                })
                .count()
        });
    SourcePipelineValidationMetrics {
        validation_target_plan_id: inputs.validation_target_plan_id.map(str::to_string),
        eligible_auto_promotion_owner_count: eligible_owner_ids.len(),
        eligible_auto_promotion_owner_ids: sort_js_default(
            eligible_owner_ids.into_iter().collect(),
        ),
        eligible_auto_promotion_owner_keys: sort_js_default(
            eligible_owner_keys.into_iter().collect(),
        ),
        weak_existing_owner_count: weak_existing_owners.len(),
        high_confidence_replacement_owner_count,
        review_queue_owner_count: inputs.review_owner_keys.len(),
        source_gap_owner_count: inputs.source_gap_owner_keys.len(),
        promoted_owner_count: inputs.promoted_owner_ids.len(),
        promoted_owner_keys: sort_js_default(inputs.promoted_owner_keys.iter().cloned().collect()),
        canonical_primary_owner_count: inputs.primary_manifest_owner_ids.len(),
        parseable_primary_owner_count: inputs.parseable_primary_owner_ids.len(),
        parsed_owner_count: inputs.parsed_owner_ids.len(),
        requirement_fingerprint_owner_count: inputs.requirement_fingerprint_owner_ids.len(),
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourcePipelineValidationReport {
    pub generated_at: String,
    pub outcome: String,
    pub passed_count: usize,
    pub failed_count: usize,
    pub metrics: SourcePipelineValidationMetrics,
    pub auto_repair: Value,
    pub checks: Vec<SourcePipelineValidationCheck>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourcePipelineParserAlignmentResult {
    pub check: SourcePipelineValidationCheck,
    pub repair: Value,
    pub canonical_parseable_source_key_count: usize,
    pub parser_output_left_only_count: usize,
    pub parser_output_right_only_count: usize,
}

pub fn build_registry_parser_alignment_result(
    parseable_entries: &[Value],
    parse_report: &Value,
    generated_parsed_blocks: &[Value],
    parseable_primary_owner_count: usize,
) -> SourcePipelineParserAlignmentResult {
    let report_owners = parse_report
        .get("owners")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let parseable_source_keys = parseable_entries
        .iter()
        .map(build_manifest_parse_key)
        .collect::<Vec<_>>();
    let canonical_parseable_source_key_count = parseable_source_keys
        .iter()
        .cloned()
        .collect::<HashSet<_>>()
        .len();
    let parsed_source_keys = report_owners
        .iter()
        .flat_map(build_report_parse_keys)
        .collect::<Vec<_>>();
    let parsed_plan_source_keys = report_owners
        .iter()
        .flat_map(build_report_plan_source_keys)
        .collect::<HashSet<_>>();
    let parsed_source_key_set = parsed_source_keys.iter().cloned().collect::<HashSet<_>>();
    let generated_source_keys = generated_parsed_blocks
        .iter()
        .flat_map(build_report_parse_keys)
        .collect::<HashSet<_>>();
    let generated_plan_source_keys = generated_parsed_blocks
        .iter()
        .flat_map(build_report_plan_source_keys)
        .collect::<HashSet<_>>();
    let missing_current = parseable_entries
        .iter()
        .filter(|entry| {
            !parsed_source_key_set.contains(&build_manifest_parse_key(entry))
                && !parsed_plan_source_keys.contains(&build_manifest_plan_source_key(entry))
        })
        .cloned()
        .collect::<Vec<_>>();
    let missing_generated = parseable_entries
        .iter()
        .filter(|entry| {
            !generated_source_keys.contains(&build_manifest_parse_key(entry))
                && !generated_plan_source_keys.contains(&build_manifest_plan_source_key(entry))
        })
        .cloned()
        .collect::<Vec<_>>();
    let missing_coverage = missing_current
        .iter()
        .chain(missing_generated.iter())
        .filter_map(|entry| entry.get("planId").filter(|value| js_truthy(value)))
        .map(super::publish_readiness::js_string)
        .collect::<Vec<_>>();
    let repair = json!({
        "needed": !missing_current.is_empty() || !missing_generated.is_empty(),
        "targetPlanIds": unique_locale_sorted(missing_coverage),
        "missingCurrentSourceCoverage": missing_current.iter().map(summarize_parseable_source_entry).collect::<Vec<_>>(),
        "missingGeneratedSourceCoverage": missing_generated.iter().map(summarize_parseable_source_entry).collect::<Vec<_>>(),
    });
    let (parser_output_left_only, parser_output_right_only) =
        compare_string_sets(&parseable_source_keys, &parsed_source_keys);
    let check = run_source_pipeline_check(
        "registry-parser-alignment",
        "Canonical parseable requirement sources align with parser input and parser output",
        || {
            if canonical_parseable_source_key_count.saturating_sub(missing_current.len())
                != canonical_parseable_source_key_count
            {
                let missing = missing_current
                    .iter()
                    .map(build_manifest_parse_key)
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "Parse report should cover every canonical parseable source owner/url, allowing canonicalized duplicate owner/source blocks: {missing}"
                ));
            }
            if !missing_current.is_empty() {
                let missing = missing_current
                    .iter()
                    .map(build_manifest_parse_key)
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "Current parseable registry source(s) missing parser coverage: {missing}"
                ));
            }
            if !missing_generated.is_empty() {
                let missing = missing_generated
                    .iter()
                    .map(build_manifest_parse_key)
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "Generated parsed-source registry missing parser coverage: {missing}"
                ));
            }
            Ok(Value::Array(vec![
                Value::String(format!(
                    "Canonical parseable primary owners: {parseable_primary_owner_count}"
                )),
                Value::String(format!(
                    "Parseable requirement sources: {}",
                    parseable_entries.len()
                )),
                Value::String(format!(
                    "Canonical parseable owner/source keys: {canonical_parseable_source_key_count}"
                )),
                Value::String(format!(
                    "Parsed source blocks: {}",
                    js_property_string(parse_report.get("totalOwners"))
                )),
                Value::String(format!(
                    "Generated parsed source blocks: {}",
                    generated_parsed_blocks.len()
                )),
                Value::String(format!(
                    "Exact parser/registry source delta: registry-only={}, parsed-only={}",
                    parser_output_left_only.len(),
                    parser_output_right_only.len()
                )),
            ]))
        },
    );
    SourcePipelineParserAlignmentResult {
        check,
        repair,
        canonical_parseable_source_key_count,
        parser_output_left_only_count: parser_output_left_only.len(),
        parser_output_right_only_count: parser_output_right_only.len(),
    }
}

fn summarize_parseable_source_entry(entry: &Value) -> Value {
    let mut summary = serde_json::Map::new();
    for field in [
        "ownerId",
        "planId",
        "pathwayId",
        "ownerType",
        "url",
        "role",
        "parserType",
    ] {
        if matches!(
            field,
            "planId" | "pathwayId" | "ownerType" | "role" | "parserType"
        ) {
            summary.insert(
                field.to_string(),
                entry.get(field).cloned().unwrap_or(Value::Null),
            );
        } else if let Some(value) = entry.get(field) {
            summary.insert(field.to_string(), value.clone());
        }
    }
    summary.insert(
        "isPrimaryDegreeRequirementsLink".to_string(),
        Value::Bool(
            entry
                .get("isPrimaryDegreeRequirementsLink")
                .is_some_and(js_truthy),
        ),
    );
    Value::Object(summary)
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourcePipelineArtifactAlignmentResult {
    pub promotion_report_check: SourcePipelineValidationCheck,
    pub promoted_owners_check: SourcePipelineValidationCheck,
    pub canonical_parse_check: SourcePipelineValidationCheck,
    pub fingerprint_alignment_check: SourcePipelineValidationCheck,
    pub promoted_owner_coverage_repair: Value,
    pub fingerprint_alignment_repair: Value,
}

pub fn build_artifact_alignment_checks(
    promotion_report: &Value,
    promotions: &[Value],
    parseable_entries: &[Value],
    parse_report: &Value,
    fingerprint_report: &Value,
    generated_fingerprints: &[Value],
) -> SourcePipelineArtifactAlignmentResult {
    let parse_owners = value_array(parse_report.get("owners"));
    let fingerprint_owner_ids = generated_fingerprints
        .iter()
        .map(|entry| js_property_string(entry.get("ownerId")))
        .collect::<HashSet<_>>();
    let parsed_owner_ids = parse_owners
        .iter()
        .map(|owner| js_property_string(owner.get("ownerId")))
        .collect::<HashSet<_>>();
    let successful_source_keys = parse_owners
        .iter()
        .filter(|owner| owner.get("ok") == Some(&Value::Bool(true)))
        .flat_map(build_report_parse_keys)
        .collect::<HashSet<_>>();
    let parseable_source_keys = parseable_entries
        .iter()
        .map(build_manifest_parse_key)
        .collect::<HashSet<_>>();
    let failed_promoted_sources = promotions
        .iter()
        .filter(|entry| {
            let key = build_manifest_parse_key(entry);
            parseable_source_keys.contains(&key) && !successful_source_keys.contains(&key)
        })
        .cloned()
        .collect::<Vec<_>>();
    let missing_promoted_parsed = promotions
        .iter()
        .filter(|entry| !parsed_owner_ids.contains(&js_property_string(entry.get("ownerId"))))
        .cloned()
        .collect::<Vec<_>>();
    let missing_promoted_fingerprints = promotions
        .iter()
        .filter(|entry| !fingerprint_owner_ids.contains(&js_property_string(entry.get("ownerId"))))
        .cloned()
        .collect::<Vec<_>>();
    let parsed_ids = parsed_owner_ids.iter().cloned().collect::<Vec<_>>();
    let fingerprint_ids = fingerprint_owner_ids.iter().cloned().collect::<Vec<_>>();
    let (parsed_only_owner_ids, fingerprint_only_owner_ids) =
        compare_string_sets(&parsed_ids, &fingerprint_ids);
    let missing_promoted_owner_ids = missing_promoted_parsed
        .iter()
        .map(|entry| js_property_string(entry.get("ownerId")))
        .collect::<Vec<_>>();
    let missing_fingerprint_owner_ids = missing_promoted_fingerprints
        .iter()
        .map(|entry| js_property_string(entry.get("ownerId")))
        .collect::<Vec<_>>();
    let promoted_target_plan_ids = missing_promoted_parsed
        .iter()
        .chain(missing_promoted_fingerprints.iter())
        .filter_map(|entry| {
            let plan_id_value = entry.get("planId").filter(|value| !value.is_null());
            if let Some(plan_id) = plan_id_value {
                return js_truthy(plan_id).then(|| super::publish_readiness::js_string(plan_id));
            }
            get_plan_id_from_owner_id(entry.get("ownerId"))
        })
        .collect::<Vec<_>>();
    let promoted_owner_coverage_repair = json!({
        "needed": !missing_promoted_parsed.is_empty() || !missing_promoted_fingerprints.is_empty(),
        "targetPlanIds": unique_locale_sorted(promoted_target_plan_ids),
        "missingParsedOwnerIds": sort_js_default(missing_promoted_owner_ids),
        "missingFingerprintOwnerIds": sort_js_default(missing_fingerprint_owner_ids),
    });
    let total_fingerprints = fingerprint_report.get("totalRequirementSourceFingerprints");
    let total_parsed_owners = parse_report.get("totalOwners");
    let fingerprint_alignment_repair = json!({
        "needed": total_fingerprints != total_parsed_owners
            || !parsed_only_owner_ids.is_empty()
            || !fingerprint_only_owner_ids.is_empty(),
        "parsedOnlyOwnerIds": parsed_only_owner_ids,
        "fingerprintOnlyOwnerIds": fingerprint_only_owner_ids,
    });

    let report_count_matches = promotion_report.get("totalPromotions")
        == Some(&Value::Number((promotions.len() as u64).into()));
    let promotion_count_check = run_source_pipeline_check(
        "promotion-report-matches-generated-registry",
        "Generated promotion registry matches the promotion report",
        || {
            if !report_count_matches {
                return Err(
                    "Promotion report count should match the generated promotion registry."
                        .to_string(),
                );
            }
            Ok(Value::String(format!(
                "Promoted owners: {}",
                js_property_string(promotion_report.get("totalPromotions"))
            )))
        },
    );
    let promoted_check = run_source_pipeline_check(
        "promoted-owners-parsed-and-fingerprinted",
        "Promoted owners appear in parser output and requirement fingerprints",
        || {
            if !failed_promoted_sources.is_empty() {
                return Err(format!(
                    "Promoted primary sources must parse successfully: {}",
                    failed_promoted_sources
                        .iter()
                        .take(12)
                        .map(build_manifest_parse_key)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if !missing_promoted_parsed.is_empty() {
                return Err(format!(
                    "Promoted owners missing parsed blocks: {}",
                    missing_promoted_parsed
                        .iter()
                        .take(12)
                        .map(|entry| js_property_string(entry.get("ownerId")))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if !missing_promoted_fingerprints.is_empty() {
                return Err(format!(
                    "Promoted owners missing requirement fingerprints: {}",
                    missing_promoted_fingerprints
                        .iter()
                        .take(12)
                        .map(|entry| js_property_string(entry.get("ownerId")))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            Ok(Value::String(format!(
                "Promoted owners verified end-to-end: {}",
                promotions.len()
            )))
        },
    );
    let canonical_parse_check = run_source_pipeline_check(
        "canonical-requirement-sources-parse-successfully",
        "Every canonical parseable requirement source parses successfully",
        || {
            let failed = find_failed_parseable_requirement_sources(parseable_entries, parse_owners);
            if !failed.is_empty() {
                return Err(format!(
                    "Canonical requirement sources must parse successfully: {}",
                    failed
                        .iter()
                        .take(12)
                        .map(build_manifest_parse_key)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            Ok(Value::String(format!(
                "Canonical parseable sources verified: {}",
                parseable_entries.len()
            )))
        },
    );
    let fingerprint_check = run_source_pipeline_check(
        "fingerprint-alignment",
        "Requirement fingerprint coverage stays aligned with parsed requirement owners",
        || {
            if total_fingerprints != total_parsed_owners {
                return Err(
                    "Requirement fingerprint count should match parsed owner count.".to_string(),
                );
            }
            if !parsed_only_owner_ids.is_empty() || !fingerprint_only_owner_ids.is_empty() {
                return Err(format!(
                    "Requirement fingerprint owner mismatch. parsed-only={} fingerprint-only={}",
                    parsed_only_owner_ids.join(", "),
                    fingerprint_only_owner_ids.join(", ")
                ));
            }
            Ok(Value::Array(vec![
                Value::String(format!(
                    "Requirement fingerprints: {}",
                    js_property_string(total_fingerprints)
                )),
                Value::String(format!(
                    "Parsed owners: {}",
                    js_property_string(total_parsed_owners)
                )),
                Value::String(format!(
                    "Touched source owners: {}",
                    value_array(fingerprint_report.get("touchedSourceOwnerIds")).len()
                )),
                Value::String(format!(
                    "Touched requirement owners: {}",
                    value_array(fingerprint_report.get("touchedRequirementOwnerIds")).len()
                )),
            ]))
        },
    );
    SourcePipelineArtifactAlignmentResult {
        promotion_report_check: promotion_count_check,
        promoted_owners_check: promoted_check,
        canonical_parse_check,
        fingerprint_alignment_check: fingerprint_check,
        promoted_owner_coverage_repair,
        fingerprint_alignment_repair,
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourcePipelineRelationshipChecks {
    pub review_queue_alignment_check: SourcePipelineValidationCheck,
    pub promotion_materialization_check: SourcePipelineValidationCheck,
}

pub fn build_review_and_promotion_checks(
    missing_primary_review_owner_keys: &HashSet<String>,
    review_owner_keys: &HashSet<String>,
    source_gap_owner_keys: &HashSet<String>,
    promotions: &[Value],
    primary_manifest_owner_ids: &HashSet<String>,
    discovery_report: &Value,
) -> SourcePipelineRelationshipChecks {
    let (review_only, source_gap_only) = compare_string_sets(
        &missing_primary_review_owner_keys
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        &source_gap_owner_keys.iter().cloned().collect::<Vec<_>>(),
    );
    let review_queue_alignment_check = run_source_pipeline_check(
        "review-queue-aligned-with-source-gaps",
        "Missing-primary review queue and source-gap report point at the same unresolved owners",
        || {
            if !review_only.is_empty() || !source_gap_only.is_empty() {
                return Err(format!(
                    "Review/source-gap mismatch. review-only={} source-gap-only={}",
                    review_only.join(", "),
                    source_gap_only.join(", ")
                ));
            }
            Ok(Value::String(format!(
                "Shared unresolved owners: {}",
                source_gap_owner_keys.len()
            )))
        },
    );

    let missing_canonical_primary = promotions
        .iter()
        .filter(|entry| {
            !primary_manifest_owner_ids.contains(&js_property_string(entry.get("ownerId")))
        })
        .collect::<Vec<_>>();
    let mut review_intersection = Vec::new();
    let mut gap_intersection = Vec::new();
    for promotion in promotions {
        let owner_key = js_property_string(promotion.get("ownerKey"));
        if review_owner_keys.contains(&owner_key) {
            let owner = value_array(discovery_report.get("weakExistingOwners"))
                .iter()
                .chain(value_array(discovery_report.get("owners")))
                .find(|owner| build_owner_key(owner) == owner_key);
            if !owner.is_some_and(current_primary_cannot_create_schedulable_rows) {
                review_intersection.push(js_property_string(promotion.get("ownerId")));
            }
        }
        if source_gap_owner_keys.contains(&owner_key) {
            gap_intersection.push(js_property_string(promotion.get("ownerId")));
        }
    }
    let review_intersection = unique_locale_sorted(review_intersection);
    let gap_intersection = unique_locale_sorted(gap_intersection);
    let promotion_materialization_check = run_source_pipeline_check(
        "promotions-materialized-in-canonical-registry",
        "Auto-promoted owners are materialized in the canonical primary-source registry",
        || {
            if !missing_canonical_primary.is_empty() {
                return Err(format!(
                    "Promoted owners missing canonical primary entries: {}",
                    missing_canonical_primary
                        .iter()
                        .map(|entry| js_property_string(entry.get("ownerId")))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if !review_intersection.is_empty() {
                return Err(format!(
                    "Promoted owners should not remain in the review queue: {}",
                    review_intersection.join(", ")
                ));
            }
            if !gap_intersection.is_empty() {
                return Err(format!(
                    "Promoted owners should not remain hidden as source gaps: {}",
                    gap_intersection.join(", ")
                ));
            }
            Ok(Value::Array(vec![
                Value::String(format!("Promoted owners: {}", promotions.len())),
                Value::String(format!(
                    "Canonical primary owners: {}",
                    primary_manifest_owner_ids.len()
                )),
            ]))
        },
    );
    SourcePipelineRelationshipChecks {
        review_queue_alignment_check,
        promotion_materialization_check,
    }
}

pub fn build_discovery_partition_check(
    validation_scoped_source_gap_discovery_owner_keys: &HashSet<String>,
    eligible_missing_primary_owners: &[Value],
    blocked_missing_primary_owners: &[Value],
    missing_primary_review_owner_keys: &HashSet<String>,
    eligible_weak_existing_replacement_owners: &[Value],
    validation_scoped_review_owner_count: usize,
) -> SourcePipelineValidationCheck {
    let eligible_count = eligible_missing_primary_owners
        .iter()
        .filter(|owner| {
            validation_scoped_source_gap_discovery_owner_keys.contains(&build_owner_key(owner))
        })
        .count();
    let blocked_count = blocked_missing_primary_owners
        .iter()
        .filter(|owner| {
            validation_scoped_source_gap_discovery_owner_keys.contains(&build_owner_key(owner))
        })
        .count();
    run_source_pipeline_check(
        "discovery-partition",
        "Missing-primary discovery owners partition cleanly into eligible, review, and blocked buckets",
        || {
            let partitioned_count =
                eligible_count + missing_primary_review_owner_keys.len() + blocked_count;
            if partitioned_count != validation_scoped_source_gap_discovery_owner_keys.len() {
                return Err("Discovery owner count should equal eligible auto-promotions plus review-queue owners plus explicitly blocked auto-promotions.".to_string());
            }
            Ok(Value::Array(vec![
                Value::String(format!(
                    "Discovery owners: {}",
                    validation_scoped_source_gap_discovery_owner_keys.len()
                )),
                Value::String(format!(
                    "Eligible missing-primary auto-promotions: {eligible_count}"
                )),
                Value::String(format!(
                    "Blocked missing-primary auto-promotions: {blocked_count}"
                )),
                Value::String(format!(
                    "Eligible weak-existing replacements: {}",
                    eligible_weak_existing_replacement_owners.len()
                )),
                Value::String(format!(
                    "Review-queue owners: {validation_scoped_review_owner_count}"
                )),
                Value::String(format!(
                    "Missing-primary review owners: {}",
                    missing_primary_review_owner_keys.len()
                )),
            ]))
        },
    )
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourcePipelineEligiblePromotionChecks {
    pub eligible_discoveries_promoted_check: SourcePipelineValidationCheck,
    pub eligible_promotions_cleared_check: SourcePipelineValidationCheck,
}

pub fn build_eligible_promotion_checks(
    validation_target_plan_id: Option<&str>,
    eligible_auto_promotion_owners: &[Value],
    promoted_owner_ids: &HashSet<String>,
    review_owner_keys: &HashSet<String>,
    source_gap_owner_keys: &HashSet<String>,
) -> SourcePipelineEligiblePromotionChecks {
    let eligible_discoveries_promoted_check = run_source_pipeline_check(
        "eligible-discoveries-promoted",
        "Eligible high-confidence discoveries are promoted unless they remain in the review queue",
        || {
            if let Some(target_plan_id) = validation_target_plan_id.filter(|id| !id.is_empty()) {
                return Ok(Value::String(format!(
                    "Skipped canonical promotion check for requested target plan: {target_plan_id}"
                )));
            }
            let missing_promotions = eligible_auto_promotion_owners
                .iter()
                .filter(|owner| {
                    !promoted_owner_ids.contains(&js_property_string(owner.get("ownerId")))
                })
                .collect::<Vec<_>>();
            if !missing_promotions.is_empty() {
                return Err(format!(
                    "Missing promoted owners: {}",
                    missing_promotions
                        .iter()
                        .take(12)
                        .map(|owner| js_property_string(owner.get("ownerId")))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            Ok(Value::String(format!(
                "Eligible promoted owners verified: {}",
                eligible_auto_promotion_owners.len()
            )))
        },
    );
    let lingering_review_owners = eligible_auto_promotion_owners
        .iter()
        .filter(|owner| review_owner_keys.contains(&js_property_string(owner.get("ownerKey"))))
        .map(|owner| js_property_string(owner.get("ownerId")))
        .collect::<Vec<_>>();
    let lingering_gap_owners = eligible_auto_promotion_owners
        .iter()
        .filter(|owner| source_gap_owner_keys.contains(&js_property_string(owner.get("ownerKey"))))
        .map(|owner| js_property_string(owner.get("ownerId")))
        .collect::<Vec<_>>();
    let eligible_promotions_cleared_check = run_source_pipeline_check(
        "eligible-promotions-cleared-from-review-and-gap-reports",
        "Eligible auto-promotions are fully cleared from the review queue and source-gap report",
        || {
            if !lingering_review_owners.is_empty() {
                return Err(format!(
                    "Eligible auto-promotions still in review queue: {}",
                    lingering_review_owners.join(", ")
                ));
            }
            if !lingering_gap_owners.is_empty() {
                return Err(format!(
                    "Eligible auto-promotions still in source-gap report: {}",
                    lingering_gap_owners.join(", ")
                ));
            }
            Ok(Value::String(format!(
                "Eligible owners fully cleared: {}",
                eligible_auto_promotion_owners.len()
            )))
        },
    );
    SourcePipelineEligiblePromotionChecks {
        eligible_discoveries_promoted_check,
        eligible_promotions_cleared_check,
    }
}

/// Compose the ten synchronous checks in the exact order used by the CJS validator.
pub fn assemble_source_pipeline_core_checks(
    discovery_partition_check: SourcePipelineValidationCheck,
    artifact_checks: SourcePipelineArtifactAlignmentResult,
    eligible_checks: SourcePipelineEligiblePromotionChecks,
    relationship_checks: SourcePipelineRelationshipChecks,
    parser_alignment: SourcePipelineParserAlignmentResult,
) -> Vec<SourcePipelineValidationCheck> {
    vec![
        discovery_partition_check,
        artifact_checks.promotion_report_check,
        eligible_checks.eligible_discoveries_promoted_check,
        relationship_checks.review_queue_alignment_check,
        relationship_checks.promotion_materialization_check,
        parser_alignment.check,
        artifact_checks.promoted_owners_check,
        artifact_checks.canonical_parse_check,
        artifact_checks.fingerprint_alignment_check,
        eligible_checks.eligible_promotions_cleared_check,
    ]
}

fn value_array(value: Option<&Value>) -> &[Value] {
    value
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn get_plan_id_from_owner_id(owner_id: Option<&Value>) -> Option<String> {
    let owner_id = owner_id
        .filter(|owner_id| !owner_id.is_null())
        .map(super::publish_readiness::js_string)
        .unwrap_or_default();
    let plan_id = owner_id.split(":pathway:").next().unwrap_or_default();
    (!plan_id.is_empty()).then(|| plan_id.to_string())
}

fn sort_js_default(mut values: Vec<String>) -> Vec<String> {
    values.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
    values
}

pub fn run_source_pipeline_check<F>(
    id: impl Into<String>,
    label: impl Into<String>,
    callback: F,
) -> SourcePipelineValidationCheck
where
    F: FnOnce() -> Result<Value, String>,
{
    let id = id.into();
    let label = label.into();
    match callback() {
        Ok(Value::Array(values)) => SourcePipelineValidationCheck {
            id,
            label,
            status: "passed".to_string(),
            details: values
                .iter()
                .map(super::publish_readiness::js_string)
                .collect(),
        },
        Ok(value) => SourcePipelineValidationCheck {
            id,
            label,
            status: "passed".to_string(),
            details: if js_truthy(&value) {
                vec![super::publish_readiness::js_string(&value)]
            } else {
                Vec::new()
            },
        },
        Err(error) => SourcePipelineValidationCheck {
            id,
            label,
            status: "failed".to_string(),
            details: vec![error],
        },
    }
}

pub async fn run_async_source_pipeline_check<F, Fut>(
    id: impl Into<String>,
    label: impl Into<String>,
    callback: F,
) -> SourcePipelineValidationCheck
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    let result = callback().await;
    run_source_pipeline_check(id, label, || result)
}

pub fn build_source_pipeline_validation_report(
    generated_at: impl Into<String>,
    metrics: SourcePipelineValidationMetrics,
    auto_repair: Value,
    checks: Vec<SourcePipelineValidationCheck>,
) -> SourcePipelineValidationReport {
    let failed_count = checks
        .iter()
        .filter(|check| check.status == "failed")
        .count();
    SourcePipelineValidationReport {
        generated_at: generated_at.into(),
        outcome: if failed_count == 0 {
            "passed".to_string()
        } else {
            "failed".to_string()
        },
        passed_count: checks.len() - failed_count,
        failed_count,
        metrics,
        auto_repair,
        checks,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourcePipelineReportArtifacts {
    pub json: String,
    pub markdown: String,
}

/// Serialize the report pair in memory with the same trailing-newline contract as the CJS writer.
/// This intentionally does not create or overwrite generated report files.
pub fn render_source_pipeline_report_artifacts(
    report: &SourcePipelineValidationReport,
) -> Result<SourcePipelineReportArtifacts, String> {
    let mut json = serde_json::to_string_pretty(report)
        .map_err(|error| format!("could not serialize source-pipeline JSON report: {error}"))?;
    json.push('\n');
    let report_value = serde_json::to_value(report)
        .map_err(|error| format!("could not prepare source-pipeline Markdown report: {error}"))?;
    Ok(SourcePipelineReportArtifacts {
        json,
        markdown: render_markdown_report(&report_value),
    })
}

/// Write the two generated reports to the CJS-compatible source-root paths. This is an explicit
/// operation; report construction and the JSON CLI remain in-memory unless a caller opts in.
pub fn write_source_pipeline_report_artifacts(
    source_root: &Path,
    report: &SourcePipelineValidationReport,
) -> Result<(PathBuf, PathBuf), String> {
    let artifacts = render_source_pipeline_report_artifacts(report)?;
    let report_dir = source_root.join(".tmp").join("reports");
    fs::create_dir_all(&report_dir).map_err(|error| {
        format!(
            "could not create source-pipeline report directory {}: {error}",
            report_dir.display()
        )
    })?;
    let json_path = report_dir.join("transfer-planner-source-pipeline-validation.json");
    let markdown_path = report_dir.join("transfer-planner-source-pipeline-validation.md");
    fs::write(&json_path, artifacts.json).map_err(|error| {
        format!(
            "could not write source-pipeline JSON report {}: {error}",
            json_path.display()
        )
    })?;
    fs::write(&markdown_path, artifacts.markdown).map_err(|error| {
        format!(
            "could not write source-pipeline Markdown report {}: {error}",
            markdown_path.display()
        )
    })?;
    Ok((json_path, markdown_path))
}

pub struct SourcePipelineValidationExecutionInput {
    pub generated_at: String,
    pub metrics: SourcePipelineValidationMetrics,
    pub discovery_partition_check: SourcePipelineValidationCheck,
    pub artifact_checks: SourcePipelineArtifactAlignmentResult,
    pub eligible_checks: SourcePipelineEligiblePromotionChecks,
    pub relationship_checks: SourcePipelineRelationshipChecks,
    pub parser_alignment: SourcePipelineParserAlignmentResult,
    pub trailing_checks: Vec<SourcePipelineValidationCheck>,
}

pub struct SourcePipelineValidationExecution {
    pub report: SourcePipelineValidationReport,
    pub artifacts: SourcePipelineReportArtifacts,
    pub exit_code: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourcePipelineJsonReports {
    pub discovery_report: Value,
    pub review_queue: Value,
    pub promotion_report: Value,
    pub source_gap_report: Value,
    pub requirement_parse_report: Value,
    pub source_fingerprint_report: Value,
}

fn read_source_pipeline_json_report(
    source_root: &Path,
    file_name: &str,
    label: &str,
) -> Result<Value, String> {
    let path = source_root.join(".tmp").join("reports").join(file_name);
    let contents =
        fs::read_to_string(&path).map_err(|_| format!("Missing {label} at {}", path.display()))?;
    serde_json::from_str(&contents)
        .map_err(|error| format!("Could not decode {label} at {}: {error}", path.display()))
}

/// Read the six report inputs used by the CJS validator without creating directories or writing
/// any generated report files.
pub fn load_source_pipeline_json_reports(
    source_root: &Path,
) -> Result<SourcePipelineJsonReports, String> {
    Ok(SourcePipelineJsonReports {
        discovery_report: read_source_pipeline_json_report(
            source_root,
            "transfer-planner-primary-source-discovery.json",
            "primary-source discovery report",
        )?,
        review_queue: read_source_pipeline_json_report(
            source_root,
            "transfer-planner-primary-source-review-queue.json",
            "primary-source review queue",
        )?,
        promotion_report: read_source_pipeline_json_report(
            source_root,
            "transfer-planner-primary-source-promotions.json",
            "primary-source promotion report",
        )?,
        source_gap_report: read_source_pipeline_json_report(
            source_root,
            "transfer-planner-source-gaps.json",
            "source-gap report",
        )?,
        requirement_parse_report: read_source_pipeline_json_report(
            source_root,
            "transfer-planner-requirement-source-parse-report.json",
            "requirement source parse report",
        )?,
        source_fingerprint_report: read_source_pipeline_json_report(
            source_root,
            "transfer-planner-source-fingerprints.json",
            "source fingerprint report",
        )?,
    })
}

pub struct SourcePipelineValidationInput<'a> {
    pub validation_target_plan_id: Option<&'a str>,
    pub discovery_report: &'a Value,
    pub review_queue: &'a Value,
    pub promotion_report: &'a Value,
    pub source_gap_report: &'a Value,
    pub requirement_parse_report: &'a Value,
    pub source_fingerprint_report: &'a Value,
    pub canonical_primary_manifest: &'a [Value],
    pub parseable_primary_manifest: &'a [Value],
    pub parseable_requirement_source_entries: &'a [Value],
    pub promotions: &'a [Value],
    pub generated_parsed_blocks: &'a [Value],
    pub generated_requirement_fingerprints: &'a [Value],
    pub parser_backed_review_owner_keys: &'a HashSet<String>,
    pub active_pathways: &'a [Value],
    pub promotion_context: &'a SourcePipelinePromotionContext<'a>,
}

fn required_report_input<'a>(input: &'a Value, key: &str) -> Result<&'a Value, String> {
    let value = input
        .get(key)
        .ok_or_else(|| format!("source-pipeline input is missing {key}"))?;
    if !value.is_object() {
        return Err(format!("source-pipeline input {key} must be an object"));
    }
    Ok(value)
}

fn required_array_input<'a>(input: &'a Value, key: &str) -> Result<&'a [Value], String> {
    input
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("source-pipeline input {key} must be an array"))
}

fn required_string_set_input(input: &Value, key: &str) -> Result<HashSet<String>, String> {
    let values = required_array_input(input, key)?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("source-pipeline input {key}[{index}] must be a string"))
        })
        .collect()
}

fn load_source_pipeline_canonical_context(
    source_root: &Path,
    discovery_report: &Value,
    review_queue: &Value,
) -> Result<Value, String> {
    let bridge_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("source-pipeline-context-bridge.cjs");
    let mut child = Command::new("node")
        .arg(&bridge_path)
        .arg(source_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not start canonical planner data bridge: {error}"))?;
    let bridge_input = serde_json::to_vec(&json!({
        "discoveryReport": discovery_report,
        "reviewQueue": review_queue,
    }))
    .map_err(|error| format!("could not encode canonical planner bridge input: {error}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| "canonical planner data bridge stdin is unavailable".to_string())?
        .write_all(&bridge_input)
        .map_err(|error| {
            format!("could not send input to canonical planner data bridge: {error}")
        })?;
    let output = child.wait_with_output().map_err(|error| {
        format!("could not collect canonical planner data bridge output: {error}")
    })?;
    if !output.status.success() {
        return Err(format!(
            "canonical planner data bridge failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("could not decode canonical planner data bridge output: {error}"))
}

/// Build the deterministic core from a JSON-compatible fixture/context. This creates no files;
/// callers can compare its report against the still-authoritative CJS validator.
pub fn build_source_pipeline_core_execution_from_json(
    input: &Value,
) -> Result<SourcePipelineValidationExecution, String> {
    let mut resolved_input = input.clone();
    let load_canonical_registry = input.get("loadCanonicalRegistry") == Some(&Value::Bool(true));
    if load_canonical_registry && input.get("sourceRoot").is_none() {
        return Err(
            "source-pipeline input sourceRoot is required to load canonical registry data"
                .to_string(),
        );
    }
    let source_root = if let Some(source_root_value) = input.get("sourceRoot") {
        let source_root = source_root_value
            .as_str()
            .ok_or_else(|| "source-pipeline input sourceRoot must be a string".to_string())?;
        let source_root = Path::new(source_root);
        let reports = load_source_pipeline_json_reports(source_root)?;
        let canonical = if load_canonical_registry {
            Some(load_source_pipeline_canonical_context(
                source_root,
                &reports.discovery_report,
                &reports.review_queue,
            )?)
        } else {
            None
        };
        let object = resolved_input
            .as_object_mut()
            .ok_or_else(|| "source-pipeline input must be an object".to_string())?;
        object.insert("discoveryReport".to_string(), reports.discovery_report);
        object.insert("reviewQueue".to_string(), reports.review_queue);
        object.insert("promotionReport".to_string(), reports.promotion_report);
        object.insert("sourceGapReport".to_string(), reports.source_gap_report);
        object.insert(
            "requirementParseReport".to_string(),
            reports.requirement_parse_report,
        );
        object.insert(
            "sourceFingerprintReport".to_string(),
            reports.source_fingerprint_report,
        );
        if let Some(canonical) = canonical {
            for key in [
                "canonicalPrimaryManifest",
                "parseablePrimaryManifest",
                "parseableRequirementSourceEntries",
                "promotions",
                "generatedParsedBlocks",
                "generatedRequirementFingerprints",
                "activePathways",
                "generatedPlans",
                "manualOverrides",
                "runtimeAliasCoveredOwnerIds",
                "runtimePathwayRecords",
                "structurallySuspiciousOwnerKeys",
            ] {
                let value = canonical
                    .get(key)
                    .ok_or_else(|| format!("canonical planner data bridge omitted {key}"))?;
                object.insert(key.to_string(), value.clone());
            }
        }
        Some(source_root)
    } else {
        None
    };
    let input = &resolved_input;
    let generated_at = input
        .get("generatedAt")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            source_root
                .map(|_| chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        })
        .ok_or_else(|| "source-pipeline input generatedAt must be a string".to_string())?;
    let validation_target_plan_id = match input.get("validationTargetPlanId") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.as_str()),
        Some(_) => {
            return Err(
                "source-pipeline input validationTargetPlanId must be a string or null".to_string(),
            );
        }
    };
    let discovery_report = required_report_input(input, "discoveryReport")?;
    let review_queue = required_report_input(input, "reviewQueue")?;
    let promotion_report = required_report_input(input, "promotionReport")?;
    let source_gap_report = required_report_input(input, "sourceGapReport")?;
    let requirement_parse_report = required_report_input(input, "requirementParseReport")?;
    let source_fingerprint_report = required_report_input(input, "sourceFingerprintReport")?;
    let canonical_primary_manifest = required_array_input(input, "canonicalPrimaryManifest")?;
    let parseable_primary_manifest = required_array_input(input, "parseablePrimaryManifest")?;
    let parseable_requirement_source_entries =
        required_array_input(input, "parseableRequirementSourceEntries")?;
    let promotions = required_array_input(input, "promotions")?;
    let generated_parsed_blocks = required_array_input(input, "generatedParsedBlocks")?;
    let generated_requirement_fingerprints =
        required_array_input(input, "generatedRequirementFingerprints")?;
    let active_pathways = required_array_input(input, "activePathways")?;
    let parser_backed_review_owner_keys =
        build_parser_backed_review_owner_keys(generated_parsed_blocks, active_pathways);
    let generated_plans = required_array_input(input, "generatedPlans")?;
    let manual_overrides = required_array_input(input, "manualOverrides")?;
    let runtime_alias_covered_owner_ids =
        required_string_set_input(input, "runtimeAliasCoveredOwnerIds")?;
    let runtime_pathway_records = required_array_input(input, "runtimePathwayRecords")?;
    let structurally_suspicious_owner_keys =
        required_string_set_input(input, "structurallySuspiciousOwnerKeys")?;
    let promotion_context = SourcePipelinePromotionContext {
        generated_plans,
        manual_overrides,
        runtime_alias_covered_owner_ids: &runtime_alias_covered_owner_ids,
        runtime_pathway_records,
        structurally_suspicious_owner_keys: &structurally_suspicious_owner_keys,
    };
    build_source_pipeline_core_execution(
        SourcePipelineValidationInput {
            validation_target_plan_id,
            discovery_report,
            review_queue,
            promotion_report,
            source_gap_report,
            requirement_parse_report,
            source_fingerprint_report,
            canonical_primary_manifest,
            parseable_primary_manifest,
            parseable_requirement_source_entries,
            promotions,
            generated_parsed_blocks,
            generated_requirement_fingerprints,
            parser_backed_review_owner_keys: &parser_backed_review_owner_keys,
            active_pathways,
            promotion_context: &promotion_context,
        },
        generated_at,
        super::source_pipeline_trigger_fixtures::build_source_pipeline_trigger_fixture_checks(),
    )
}

/// Run the ten deterministic report checks from already-loaded canonical inputs. Source loading
/// and external replacement-discovery fixtures remain separate so this function cannot write
/// generated files or silently substitute partial registry data.
pub fn build_source_pipeline_core_execution(
    input: SourcePipelineValidationInput<'_>,
    generated_at: impl Into<String>,
    trailing_checks: Vec<SourcePipelineValidationCheck>,
) -> Result<SourcePipelineValidationExecution, String> {
    let scoped = build_source_pipeline_scoped_owner_sets(
        input.discovery_report,
        input.review_queue,
        input.source_gap_report,
        input.validation_target_plan_id,
        input.parser_backed_review_owner_keys,
        input.active_pathways,
        input.promotion_context,
    );
    let eligible_auto_promotion_owners = get_eligible_auto_promotion_owners(
        input.discovery_report,
        input.review_queue,
        input.validation_target_plan_id,
        input.promotion_context,
    );
    let eligible_missing_primary_owners = get_eligible_missing_primary_auto_promotion_owners(
        input.discovery_report,
        input.review_queue,
        input.validation_target_plan_id,
        input.promotion_context,
    );
    let blocked_missing_primary_owners = get_blocked_missing_primary_auto_promotion_owners(
        input.discovery_report,
        input.review_queue,
        input.validation_target_plan_id,
        input.promotion_context,
    );
    let eligible_weak_replacements = get_eligible_weak_existing_replacement_owners(
        input.discovery_report,
        input.review_queue,
        input.validation_target_plan_id,
        input.promotion_context,
    );
    let promoted_owner_ids = input
        .promotions
        .iter()
        .map(|entry| js_property_string(entry.get("ownerId")))
        .collect::<HashSet<_>>();
    let promoted_owner_keys = input
        .promotions
        .iter()
        .map(|entry| js_property_string(entry.get("ownerKey")))
        .collect::<HashSet<_>>();
    let primary_manifest_owner_ids =
        build_primary_manifest_owner_ids(input.canonical_primary_manifest);
    let parseable_primary_owner_ids =
        build_parseable_primary_manifest_owner_ids(input.parseable_primary_manifest);
    let parsed_owner_ids = value_array(input.requirement_parse_report.get("owners"))
        .iter()
        .map(|owner| js_property_string(owner.get("ownerId")))
        .collect::<HashSet<_>>();
    let requirement_fingerprint_owner_ids = input
        .generated_requirement_fingerprints
        .iter()
        .map(|entry| js_property_string(entry.get("ownerId")))
        .collect::<HashSet<_>>();
    let artifact_checks = build_artifact_alignment_checks(
        input.promotion_report,
        input.promotions,
        input.parseable_requirement_source_entries,
        input.requirement_parse_report,
        input.source_fingerprint_report,
        input.generated_requirement_fingerprints,
    );
    let parser_alignment = build_registry_parser_alignment_result(
        input.parseable_requirement_source_entries,
        input.requirement_parse_report,
        input.generated_parsed_blocks,
        parseable_primary_owner_ids.len(),
    );
    let relationship_checks = build_review_and_promotion_checks(
        &scoped.validation_scoped_missing_primary_review_owner_keys,
        &scoped.review_owner_keys,
        &scoped.source_gap_owner_keys,
        input.promotions,
        &primary_manifest_owner_ids,
        input.discovery_report,
    );
    let discovery_partition_check = build_discovery_partition_check(
        &scoped.validation_scoped_source_gap_discovery_owner_keys,
        &eligible_missing_primary_owners,
        &blocked_missing_primary_owners,
        &scoped.validation_scoped_missing_primary_review_owner_keys,
        &eligible_weak_replacements,
        scoped.validation_scoped_review_owner_count,
    );
    let eligible_checks = build_eligible_promotion_checks(
        input.validation_target_plan_id,
        &eligible_auto_promotion_owners,
        &promoted_owner_ids,
        &scoped.review_owner_keys,
        &scoped.source_gap_owner_keys,
    );
    let metrics = build_source_pipeline_validation_metrics(SourcePipelineValidationMetricInputs {
        validation_target_plan_id: input.validation_target_plan_id,
        eligible_auto_promotion_owners: &eligible_auto_promotion_owners,
        discovery_report: input.discovery_report,
        review_owner_keys: &scoped.review_owner_keys,
        source_gap_owner_keys: &scoped.source_gap_owner_keys,
        promoted_owner_ids: &promoted_owner_ids,
        promoted_owner_keys: &promoted_owner_keys,
        primary_manifest_owner_ids: &primary_manifest_owner_ids,
        parseable_primary_owner_ids: &parseable_primary_owner_ids,
        parsed_owner_ids: &parsed_owner_ids,
        requirement_fingerprint_owner_ids: &requirement_fingerprint_owner_ids,
    });
    Ok(build_source_pipeline_validation_execution(
        SourcePipelineValidationExecutionInput {
            generated_at: generated_at.into(),
            metrics,
            discovery_partition_check,
            artifact_checks,
            eligible_checks,
            relationship_checks,
            parser_alignment,
            trailing_checks,
        },
    )?)
}

/// Compose the synchronous audit, append later fixture checks, and build the in-memory report
/// artifacts and CJS-compatible process status. No report files are written here.
pub fn build_source_pipeline_validation_execution(
    input: SourcePipelineValidationExecutionInput,
) -> Result<SourcePipelineValidationExecution, String> {
    let auto_repair = json!({
        "registryParserAlignment": input.parser_alignment.repair,
        "promotedOwnerCoverage": input.artifact_checks.promoted_owner_coverage_repair,
        "fingerprintAlignment": input.artifact_checks.fingerprint_alignment_repair,
    });
    let mut checks = assemble_source_pipeline_core_checks(
        input.discovery_partition_check,
        input.artifact_checks,
        input.eligible_checks,
        input.relationship_checks,
        input.parser_alignment,
    );
    checks.extend(input.trailing_checks);
    let report = build_source_pipeline_validation_report(
        input.generated_at,
        input.metrics,
        auto_repair,
        checks,
    );
    let artifacts = render_source_pipeline_report_artifacts(&report)?;
    let exit_code = i32::from(report.failed_count != 0);
    Ok(SourcePipelineValidationExecution {
        report,
        artifacts,
        exit_code,
    })
}

const TACOMA_LEGACY_SOURCE_URL_REPLACEMENTS: &[(&str, &str)] = &[
    (
        "https://www.tacoma.uw.edu/sias-new/socs-new/general-option",
        "https://www.tacoma.uw.edu/sias/socs/general-history-option",
    ),
    (
        "https://www.tacoma.uw.edu/sias-new/socs-new/arts-culture-and-society-history-option",
        "https://www.tacoma.uw.edu/sias/socs/arts-culture-and-society-option",
    ),
    (
        "https://www.tacoma.uw.edu/sias-new/socs-new/ethnic-studies-option",
        "https://www.tacoma.uw.edu/sias/socs/ethnic-studies-option",
    ),
    (
        "https://www.tacoma.uw.edu/sias-new/socs-new/global-history-option",
        "https://www.tacoma.uw.edu/sias/socs/global-history-option",
    ),
    (
        "https://www.tacoma.uw.edu/sias-new/socs-new/labor-and-social-movements-option",
        "https://www.tacoma.uw.edu/sias/socs/labor-and-social-movements-option",
    ),
    (
        "https://www.tacoma.uw.edu/sias-new/socs-new/power-gender-and-identity-option",
        "https://www.tacoma.uw.edu/sias/socs/power-gender-and-identity-option",
    ),
    (
        "https://www.tacoma.uw.edu/sias-new/cac-new/rhetoric-writing-and-social-change-track",
        "https://www.tacoma.uw.edu/sias/cac/rhetoric-writing-and-social-change-track",
    ),
];

/// Parse the source-pipeline validator's single-plan selection flags.
pub fn parse_target_plan_ids(args: &[String]) -> Result<Vec<String>, String> {
    let mut target_plan_ids = get_arg_values(args, &["--target-plan-id", "--plan-id"]);
    for value in get_arg_values(args, &["--target-plan-ids"]) {
        target_plan_ids.extend(
            value
                .split(',')
                .map(|part| trim_ecmascript_whitespace(part).to_string()),
        );
    }
    let target_plan_ids = unique_locale_sorted(
        target_plan_ids
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect(),
    );
    if target_plan_ids.len() > 1 {
        return Err(format!(
            "Source pipeline validation supports one target plan at a time; received {}.",
            target_plan_ids.join(", ")
        ));
    }
    Ok(target_plan_ids)
}

/// Render the source-pipeline report's Markdown companion without touching generated report files.
pub fn render_markdown_report(report: &Value) -> String {
    let metrics = report.get("metrics").unwrap_or(&Value::Null);
    let validation_target = metrics
        .get("validationTargetPlanId")
        .filter(|value| !value.is_null())
        .map(super::publish_readiness::js_string)
        .unwrap_or_else(|| "all owners".to_string());
    let mut lines = vec![
        "# Transfer Planner Source Pipeline Validation".to_string(),
        String::new(),
        format!(
            "Generated: {}",
            js_property_string(report.get("generatedAt"))
        ),
        String::new(),
        format!("- Outcome: {}", js_property_string(report.get("outcome"))),
        format!(
            "- Passed checks: {}",
            js_property_string(report.get("passedCount"))
        ),
        format!(
            "- Failed checks: {}",
            js_property_string(report.get("failedCount"))
        ),
        format!("- Validation target: {validation_target}"),
        format!(
            "- Eligible auto-promotions from discovery: {}",
            js_property_string(metrics.get("eligibleAutoPromotionOwnerCount"))
        ),
        format!(
            "- Weak existing primaries re-evaluated: {}",
            js_property_string(metrics.get("weakExistingOwnerCount"))
        ),
        format!(
            "- High-confidence replacements: {}",
            js_property_string(metrics.get("highConfidenceReplacementOwnerCount"))
        ),
        format!(
            "- Review-queue owners: {}",
            js_property_string(metrics.get("reviewQueueOwnerCount"))
        ),
        format!(
            "- Source-gap owners: {}",
            js_property_string(metrics.get("sourceGapOwnerCount"))
        ),
        format!(
            "- Promoted owners in canonical registry: {}",
            js_property_string(metrics.get("promotedOwnerCount"))
        ),
        format!(
            "- Parseable primary owners: {}",
            js_property_string(metrics.get("parseablePrimaryOwnerCount"))
        ),
        format!(
            "- Parsed owners: {}",
            js_property_string(metrics.get("parsedOwnerCount"))
        ),
        format!(
            "- Requirement fingerprints: {}",
            js_property_string(metrics.get("requirementFingerprintOwnerCount"))
        ),
        String::new(),
        "| Check | Status | Details |".to_string(),
        "| --- | --- | --- |".to_string(),
    ];
    if let Some(checks) = report.get("checks").and_then(Value::as_array) {
        for check in checks {
            let details = check
                .get("details")
                .and_then(Value::as_array)
                .map(|details| {
                    details
                        .iter()
                        .map(|detail| {
                            escape_markdown(&super::publish_readiness::js_string_nullish(Some(
                                detail,
                            )))
                        })
                        .collect::<Vec<_>>()
                        .join("<br>")
                })
                .unwrap_or_default();
            lines.push(format!(
                "| {} | {} | {} |",
                escape_markdown(&super::publish_readiness::js_string_nullish(
                    check.get("label")
                )),
                js_property_string(check.get("status")),
                details
            ));
        }
    }
    lines.push(String::new());
    format!("{}\n", lines.join("\n"))
}

fn escape_markdown(value: &str) -> String {
    value.replace('\\', "\\\\").replace('|', "\\|")
}

fn get_arg_values(args: &[String], flags: &[&str]) -> Vec<String> {
    let mut values = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        for flag in flags {
            let direct_prefix = format!("{flag}=");
            if let Some(value) = argument.strip_prefix(&direct_prefix) {
                let value = trim_ecmascript_whitespace(value);
                if !value.is_empty() {
                    values.push(value.to_string());
                }
                break;
            }
            if argument == flag {
                if let Some(value) = args
                    .get(index + 1)
                    .filter(|value| !value.is_empty() && !value.starts_with("--"))
                {
                    values.push(trim_ecmascript_whitespace(value).to_string());
                    index += 1;
                }
                break;
            }
        }
        index += 1;
    }
    values
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect()
}

/// Match the source-pipeline validator's URL equivalence, including the historical Tacoma aliases.
pub fn normalize_requirement_source_url_for_coverage(value: Option<&Value>) -> String {
    let raw = super::publish_readiness::js_string_nullish(value);
    let normalized = match Url::parse(&raw) {
        Ok(mut url) => {
            if url
                .host_str()
                .is_some_and(|host| host.ends_with("washington.edu"))
            {
                let _ = url.set_scheme("https");
            }
            url.set_query(None);
            url.set_fragment(None);
            let path = url.path().trim_end_matches('/').to_string();
            url.set_path(&path);
            url.to_string().to_lowercase()
        }
        Err(_) => {
            let trimmed = trim_ecmascript_whitespace(&raw);
            let without_suffix = trimmed
                .find(['?', '#'])
                .map_or(trimmed, |index| &trimmed[..index]);
            without_suffix.trim_end_matches('/').to_lowercase()
        }
    };
    TACOMA_LEGACY_SOURCE_URL_REPLACEMENTS
        .iter()
        .find_map(|(old, new)| (normalized == *old).then(|| (*new).to_string()))
        .unwrap_or(normalized)
}

pub fn build_parseable_requirement_source_manifest_entries(
    manifest: &[Value],
    bootstrap_plan_titles: &HashMap<String, String>,
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let primary_major_titles = build_primary_major_titles_by_plan_id(manifest);
    let specialized = filter_manifest_entries_for_specialized_plan_title(
        manifest,
        bootstrap_plan_titles,
        &primary_major_titles,
        course_parser,
    );
    let parseable = preselect_parseable_manifest_entries(&specialized, None);
    let mut entries = exclude_unhashed_primary_manifest_duplicates(&parseable);
    entries.sort_by(|left, right| {
        locale_like_compare(
            &super::publish_readiness::js_string_nullish(left.get("ownerTitle")),
            &super::publish_readiness::js_string_nullish(right.get("ownerTitle")),
        )
    });
    entries
}

pub fn normalize_promotion_url_for_comparison(value: Option<&Value>) -> String {
    let raw = value
        .filter(|value| !value.is_null())
        .map(super::publish_readiness::js_string)
        .unwrap_or_default();
    let normalized = match Url::parse(trim_ecmascript_whitespace(&raw)) {
        Ok(mut url) => {
            url.set_fragment(None);
            url.set_query(None);
            let pathname = url.path().trim_end_matches('/').to_string();
            url.set_path(&pathname);
            url.to_string().to_lowercase()
        }
        Err(_) => {
            let trimmed = trim_ecmascript_whitespace(&raw);
            let without_suffix = trimmed
                .find(['?', '#'])
                .map_or(trimmed, |index| &trimmed[..index]);
            without_suffix.trim_end_matches('/').to_lowercase()
        }
    };
    normalized
}

pub fn get_single_specialized_plan_official_url<'a>(
    plan_id: Option<&str>,
    generated_plans: &'a [Value],
) -> Option<String> {
    let plan_id = plan_id?;
    let plan = generated_plans
        .iter()
        .rev()
        .find(|plan| plan.get("id").and_then(Value::as_str) == Some(plan_id))?;
    if !super::publish_readiness::js_string_nullish(plan.get("title")).contains(':') {
        return None;
    }
    let mut unique_urls = HashSet::new();
    for link in value_array(plan.get("officialLinks")) {
        let normalized = normalize_promotion_url_for_comparison(link.get("url"));
        if !normalized.is_empty() {
            unique_urls.insert(normalized);
        }
    }
    (unique_urls.len() == 1).then(|| unique_urls.into_iter().next().unwrap())
}

pub fn suggested_primary_matches_specialized_plan_source(
    owner: &Value,
    generated_plans: &[Value],
) -> bool {
    let plan_id = owner.get("planId").and_then(Value::as_str);
    let Some(official_url) = get_single_specialized_plan_official_url(plan_id, generated_plans)
    else {
        return true;
    };
    let suggested_url = normalize_promotion_url_for_comparison(
        owner
            .get("suggestedPrimary")
            .and_then(|candidate| candidate.get("url")),
    );
    suggested_url.is_empty() || suggested_url == official_url
}

pub fn build_promotion_entry_candidate_from_owner(owner: &Value) -> Value {
    let pathway_id = owner.get("pathwayId").filter(|value| !value.is_null());
    let plan_id = owner.get("planId").filter(|value| !value.is_null());
    let plan_id_string = plan_id.map(super::publish_readiness::js_string);
    let pathway_id_string = pathway_id.map(super::publish_readiness::js_string);
    let owner_id = super::source_fingerprints::build_transfer_planner_owner_id(
        plan_id_string.as_deref(),
        pathway_id_string.as_deref(),
    );
    let suggested = owner.get("suggestedPrimary");
    let label = ["label", "anchorText", "pageTitle"]
        .iter()
        .filter_map(|field| suggested.and_then(|candidate| candidate.get(*field)))
        .find(|value| js_truthy(value))
        .cloned()
        .unwrap_or_else(|| {
            Value::String(format!(
                "{} requirements",
                owner
                    .get("title")
                    .filter(|value| !value.is_null())
                    .map(super::publish_readiness::js_string)
                    .unwrap_or_default()
            ))
        });
    let mut candidate = serde_json::Map::new();
    for field in ["ownerType"] {
        if let Some(value) = owner.get(field) {
            candidate.insert(field.to_string(), value.clone());
        }
    }
    candidate.insert("ownerId".to_string(), Value::String(owner_id));
    if let Some(value) = owner.get("planId") {
        candidate.insert("planId".to_string(), value.clone());
    }
    candidate.insert(
        "pathwayId".to_string(),
        pathway_id.cloned().unwrap_or(Value::Null),
    );
    for field in ["title", "campusId"] {
        if let Some(value) = owner.get(field) {
            candidate.insert(
                if field == "title" {
                    "ownerTitle".to_string()
                } else {
                    field.to_string()
                },
                value.clone(),
            );
        }
    }
    if let Some(url) = suggested.and_then(|candidate| candidate.get("url")) {
        candidate.insert("url".to_string(), url.clone());
    }
    candidate.insert("label".to_string(), label);
    Value::Object(candidate)
}

pub fn should_skip_manual_override_promotion(
    plan_id: Option<&Value>,
    pathway_id: Option<&Value>,
    url: Option<&Value>,
    manual_overrides: &[Value],
) -> bool {
    let owner_key = manual_source_owner_key(plan_id, pathway_id);
    let Some(override_entry) = manual_overrides.iter().find(|entry| {
        manual_source_owner_key(entry.get("planId"), entry.get("pathwayId")) == owner_key
    }) else {
        return false;
    };
    let normalized_url_value = super::publish_readiness::js_string_nullish(url);
    let normalized_url = trim_ecmascript_whitespace(&normalized_url_value).to_string();
    if normalized_url.is_empty() {
        return false;
    }
    if override_entry
        .get("mode")
        .filter(|mode| !mode.is_null())
        .is_some_and(|mode| mode == "replace")
    {
        return true;
    }
    let preferred_url_value =
        super::publish_readiness::js_string_nullish(override_entry.get("preferredPrimaryUrl"));
    let preferred_url = trim_ecmascript_whitespace(&preferred_url_value);
    if !preferred_url.is_empty() && normalized_url != preferred_url {
        return true;
    }
    value_array(override_entry.get("removedUrls"))
        .iter()
        .any(|removed| {
            trim_ecmascript_whitespace(&super::publish_readiness::js_string_nullish(Some(removed)))
                == normalized_url
        })
}

fn manual_source_owner_key(plan_id: Option<&Value>, pathway_id: Option<&Value>) -> String {
    let plan_id = plan_id
        .filter(|value| !value.is_null())
        .map(super::publish_readiness::js_string)
        .unwrap_or_default();
    let pathway_id = pathway_id
        .filter(|value| !value.is_null())
        .map(super::publish_readiness::js_string)
        .unwrap_or_default();
    format!(
        "{}::{}",
        trim_ecmascript_whitespace(&plan_id),
        trim_ecmascript_whitespace(&pathway_id)
    )
}

pub fn is_clearly_support_only_promotion_entry(entry: &Value) -> bool {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let text = format!(
        "{} {}",
        super::publish_readiness::js_string_nullish(entry.get("label")),
        super::publish_readiness::js_string_nullish(entry.get("url"))
    );
    PATTERN
        .get_or_init(|| {
            Regex::new(r"(?i)(?-u:\b)(advising|adviser|advisor|study abroad|support sources?|student resources?|student support|forms?|petitions?|policies|policy[-\s]*(?:procedures?|resources?|forms?)|faq|frequently asked questions)(?-u:\b)")
                .expect("valid support-only promotion pattern")
        })
        .is_match(&text)
}

pub fn has_document_url_with_appended_path(entry: &Value) -> bool {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN
        .get_or_init(|| {
            Regex::new(r"(?i)\.(?:pdf|docx?)(?:/|%2f)[^?#]").expect("valid document URL pattern")
        })
        .is_match(&super::publish_readiness::js_string_nullish(
            entry.get("url"),
        ))
}

pub fn is_minor_credential_promotion_for_major_owner(entry: &Value) -> bool {
    if entry.get("ownerType").and_then(Value::as_str) != Some("major")
        || entry.get("pathwayId").is_some_and(js_truthy)
    {
        return false;
    }
    static URL_PATTERN: OnceLock<Regex> = OnceLock::new();
    static LABEL_PATTERN: OnceLock<Regex> = OnceLock::new();
    URL_PATTERN
        .get_or_init(|| {
            Regex::new(r"(?i)#(?:credential|program)-").expect("valid credential URL pattern")
        })
        .is_match(&super::publish_readiness::js_string_nullish(
            entry.get("url"),
        ))
        && LABEL_PATTERN
            .get_or_init(|| {
                Regex::new(r"(?i)(?-u:\b)minor(?-u:\b)").expect("valid minor label pattern")
            })
            .is_match(&super::publish_readiness::js_string_nullish(
                entry.get("label"),
            ))
}

pub fn is_skip_navigation_promotion_entry(entry: &Value) -> bool {
    static TEXT_PATTERN: OnceLock<Regex> = OnceLock::new();
    static URL_PATTERN: OnceLock<Regex> = OnceLock::new();
    let text = format!(
        "{} {}",
        super::publish_readiness::js_string_nullish(entry.get("label")),
        super::publish_readiness::js_string_nullish(entry.get("url"))
    )
    .to_lowercase();
    TEXT_PATTERN
        .get_or_init(|| {
            Regex::new(r"(?-u:\b)skip\s+to\s+(?:main\s+)?content(?-u:\b)")
                .expect("valid skip-navigation text pattern")
        })
        .is_match(&text)
        || URL_PATTERN
            .get_or_init(|| {
                Regex::new(r"(?i)#content(?:$|[?&])").expect("valid skip-navigation URL pattern")
            })
            .is_match(&super::publish_readiness::js_string_nullish(
                entry.get("url"),
            ))
}

fn text_has_acs_certified_route(value: &str) -> bool {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN
        .get_or_init(|| {
            Regex::new(r"(?i)(?-u:\b)acs(?:[-_\s]?(?:certified|[0-9]{2,4}))?(?-u:\b)|(?-u:\b)american chemical society(?-u:\b)")
                .expect("valid ACS route pattern")
        })
        .is_match(value)
}

pub fn is_unscoped_acs_route_promotion_for_broad_major_owner(entry: &Value) -> bool {
    if entry.get("ownerType").and_then(Value::as_str) != Some("major")
        || entry.get("pathwayId").is_some_and(js_truthy)
    {
        return false;
    }
    let source_text = format!(
        "{} {}",
        super::publish_readiness::js_string_nullish(entry.get("label")),
        super::publish_readiness::js_string_nullish(entry.get("url"))
    );
    if !text_has_acs_certified_route(&source_text) {
        return false;
    }
    let owner_text = ["ownerId", "planId", "ownerTitle"]
        .iter()
        .map(|field| super::publish_readiness::js_string_nullish(entry.get(field)))
        .collect::<Vec<_>>()
        .join(" ");
    !text_has_acs_certified_route(&owner_text)
}

fn get_ba_bs_degree_kind(values: &[Option<&Value>]) -> Option<&'static str> {
    static BA_PATTERN: OnceLock<Regex> = OnceLock::new();
    static BS_PATTERN: OnceLock<Regex> = OnceLock::new();
    let searchable = values
        .iter()
        .filter_map(|value| *value)
        .filter(|value| js_truthy(value))
        .map(super::publish_readiness::js_string)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let searchable = searchable
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>();
    if BA_PATTERN
        .get_or_init(|| {
            Regex::new(r"(?-u:\b)(?:b\s*a|ba|bachelor\s+of\s+arts)(?-u:\b)")
                .expect("valid BA route pattern")
        })
        .is_match(&searchable)
    {
        return Some("ba");
    }
    if BS_PATTERN
        .get_or_init(|| {
            Regex::new(r"(?-u:\b)(?:b\s*s|bs|bachelor\s+of\s+science)(?-u:\b)")
                .expect("valid BS route pattern")
        })
        .is_match(&searchable)
    {
        return Some("bs");
    }
    None
}

fn pathway_has_explicit_ba_bs_degree_route(pathway: &Value) -> bool {
    static ROUTE_PATTERN: OnceLock<Regex> = OnceLock::new();
    static BACHELOR_PATTERN: OnceLock<Regex> = OnceLock::new();
    static OPTION_PATTERN: OnceLock<Regex> = OnceLock::new();
    static EXACT_PATTERN: OnceLock<Regex> = OnceLock::new();
    let text = format!(
        "{} {}",
        super::publish_readiness::js_string_nullish(pathway.get("id")),
        super::publish_readiness::js_string_nullish(pathway.get("label"))
    )
    .to_lowercase()
    .chars()
    .map(|character| {
        if character.is_ascii_alphanumeric() {
            character
        } else {
            ' '
        }
    })
    .collect::<String>();
    ROUTE_PATTERN
        .get_or_init(|| {
            Regex::new(r"(?-u:\b)(?:ba|bs|bachelor of arts|bachelor of science)\s+route(?-u:\b)")
                .expect("valid BA/BS route label")
        })
        .is_match(&text)
        || BACHELOR_PATTERN
            .get_or_init(|| {
                Regex::new(r"(?-u:\b)bachelor of (?:arts|science)(?-u:\b)")
                    .expect("valid bachelor route label")
            })
            .is_match(&text)
        || OPTION_PATTERN
            .get_or_init(|| {
                Regex::new(r"(?-u:\b)(?:ba|bs)\s+option(?-u:\b)").expect("valid BA/BS option label")
            })
            .is_match(&text)
        || EXACT_PATTERN
            .get_or_init(|| {
                Regex::new(r"^(?:ba|bs|bachelor of arts|bachelor of science)$")
                    .expect("valid exact BA/BS label")
            })
            .is_match(trim_ecmascript_whitespace(&text))
}

pub fn is_ba_bs_route_promotion_for_broad_major_owner(
    entry: &Value,
    generated_plans: &[Value],
) -> bool {
    if entry.get("ownerType").and_then(Value::as_str) != Some("major")
        || entry.get("pathwayId").is_some_and(js_truthy)
        || get_ba_bs_degree_kind(&[entry.get("label"), entry.get("url")]).is_none()
    {
        return false;
    }
    let Some(plan_id) = entry.get("planId").and_then(Value::as_str) else {
        return false;
    };
    let Some(plan) = generated_plans
        .iter()
        .rev()
        .find(|plan| plan.get("id").and_then(Value::as_str) == Some(plan_id))
    else {
        return false;
    };
    value_array(plan.get("pathways"))
        .iter()
        .any(pathway_has_explicit_ba_bs_degree_route)
}

pub fn is_tacoma_global_studies_leaked_promotion_entry(entry: &Value) -> bool {
    if entry.get("campusId").and_then(Value::as_str) != Some("uw-tacoma")
        || entry.get("planId").and_then(Value::as_str) == Some("uw-tacoma-global-studies")
    {
        return false;
    }
    let plan_id = entry.get("planId").and_then(Value::as_str);
    let pathway_id = entry.get("pathwayId").and_then(Value::as_str);
    super::source_fingerprints::normalize_transfer_planner_pathway_id(plan_id, pathway_id)
        .as_deref()
        == Some("global-studies-concentration")
}

pub fn is_seattle_jsis_asia_concentration_leaked_promotion_entry(entry: &Value) -> bool {
    static PATHWAY_PATTERN: OnceLock<Regex> = OnceLock::new();
    static PAGE_PATTERN: OnceLock<Regex> = OnceLock::new();
    static ASIAN_STUDIES_PATTERN: OnceLock<Regex> = OnceLock::new();
    if entry.get("campusId").and_then(Value::as_str) != Some("uw-seattle")
        || entry.get("planId").and_then(Value::as_str) == Some("uw-seattle-asian-studies")
    {
        return false;
    }
    let pathway_id = super::source_fingerprints::normalize_transfer_planner_pathway_id(
        entry.get("planId").and_then(Value::as_str),
        entry.get("pathwayId").and_then(Value::as_str),
    );
    if !pathway_id.as_deref().is_some_and(|pathway| {
        PATHWAY_PATTERN
            .get_or_init(|| {
                Regex::new(
                    r"(?i)^(?:china|general|japan|korea|south-asia|southeast-asia)-concentration$",
                )
                .expect("valid JSIS concentration pattern")
            })
            .is_match(pathway)
    }) {
        return false;
    }
    let text = ["label", "url", "ownerTitle"]
        .iter()
        .map(|field| super::publish_readiness::js_string_nullish(entry.get(field)))
        .collect::<Vec<_>>()
        .join(" ");
    PAGE_PATTERN
        .get_or_init(|| {
            Regex::new(r"(?i)/programs/undergraduate/asia-studies/")
                .expect("valid Asia Studies URL pattern")
        })
        .is_match(&text)
        || ASIAN_STUDIES_PATTERN
            .get_or_init(|| {
                Regex::new(r"(?i)(?-u:\b)Asian Studies(?-u:\b)")
                    .expect("valid Asian Studies label pattern")
            })
            .is_match(&text)
}

fn promotion_plan_for_entry<'a>(entry: &Value, generated_plans: &'a [Value]) -> Option<&'a Value> {
    let plan_id = entry.get("planId").and_then(Value::as_str)?;
    generated_plans
        .iter()
        .rev()
        .find(|plan| plan.get("id").and_then(Value::as_str) == Some(plan_id))
}

fn promotion_plan_title<'a>(entry: &Value, generated_plans: &'a [Value]) -> String {
    promotion_plan_for_entry(entry, generated_plans)
        .and_then(|plan| plan.get("title"))
        .filter(|title| !title.is_null())
        .or_else(|| entry.get("ownerTitle").filter(|title| !title.is_null()))
        .map(super::publish_readiness::js_string)
        .unwrap_or_default()
}

pub fn normalize_catalog_credential_major_name(value: Option<&Value>) -> String {
    static PARENTHESES_PATTERN: OnceLock<Regex> = OnceLock::new();
    static STOPWORDS_PATTERN: OnceLock<Regex> = OnceLock::new();
    static NON_ALPHANUMERIC_PATTERN: OnceLock<Regex> = OnceLock::new();
    let text = super::publish_readiness::js_string_nullish(value);
    let text = PARENTHESES_PATTERN
        .get_or_init(|| Regex::new(r"\([^)]*\)").expect("valid parenthetical pattern"))
        .replace_all(&text, " ")
        .replace('&', " and ");
    let text = STOPWORDS_PATTERN
        .get_or_init(|| {
            Regex::new(r"(?i)(?-u:\b)(?:bachelor|degree|with|major|minor|option|concentration|track|route|pathway|of|in|the|a|an)(?-u:\b)")
                .expect("valid catalog major stopword pattern")
        })
        .replace_all(&text, " ");
    let text = text.to_lowercase();
    let normalized = NON_ALPHANUMERIC_PATTERN
        .get_or_init(|| {
            Regex::new(r"[^a-z0-9]+").expect("valid catalog major normalization pattern")
        })
        .replace_all(&text, " ");
    trim_ecmascript_whitespace(&normalized).to_string()
}

fn promotion_plan_base_title(entry: &Value, generated_plans: &[Value]) -> String {
    static DASH_SEPARATOR_PATTERN: OnceLock<Regex> = OnceLock::new();
    static PARENTHESES_PATTERN: OnceLock<Regex> = OnceLock::new();
    let plan_title = promotion_plan_title(entry, generated_plans);
    let first_title_part = DASH_SEPARATOR_PATTERN
        .get_or_init(|| Regex::new(r"\s+-\s+").expect("valid plan title separator pattern"))
        .split(&plan_title)
        .next()
        .unwrap_or_default();
    let without_parentheticals = PARENTHESES_PATTERN
        .get_or_init(|| Regex::new(r"\([^)]*\)").expect("valid plan title parentheses pattern"))
        .replace_all(first_title_part, " ");
    let base_title = without_parentheticals.split(':').next().unwrap_or_default();
    trim_ecmascript_whitespace(base_title).to_string()
}

fn promotion_plan_comparable_titles(entry: &Value, generated_plans: &[Value]) -> Vec<String> {
    let mut titles = vec![promotion_plan_base_title(entry, generated_plans)];
    if entry.get("planId").and_then(Value::as_str) == Some("uw-seattle-european-studies") {
        titles.push("International Studies: Europe".to_string());
    }
    titles
        .iter()
        .map(|title| normalize_catalog_credential_major_name(Some(&Value::String(title.clone()))))
        .filter(|title| !title.is_empty())
        .collect()
}

fn extract_catalog_credential_major_name(label: Option<&Value>) -> Option<String> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let label = super::publish_readiness::js_string_nullish(label);
    PATTERN
        .get_or_init(|| {
            Regex::new(r"(?i)(?-u:\b)major\s+in\s+([^:()]+?)(?::|$)")
                .expect("valid catalog credential label pattern")
        })
        .captures(&label)
        .and_then(|capture| capture.get(1))
        .map(|capture| trim_ecmascript_whitespace(capture.as_str()).to_string())
}

fn extract_catalog_program_major_name(label: Option<&Value>) -> Option<String> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let label = super::publish_readiness::js_string_nullish(label);
    PATTERN
        .get_or_init(|| {
            Regex::new(r"(?i)^Program of Study:\s*Major:\s*(.+)$")
                .expect("valid catalog program label pattern")
        })
        .captures(&label)
        .and_then(|capture| capture.get(1))
        .map(|capture| trim_ecmascript_whitespace(capture.as_str()).to_string())
}

pub fn is_catalog_credential_promotion_entry(entry: &Value) -> bool {
    static URL_PATTERN: OnceLock<Regex> = OnceLock::new();
    static LABEL_PATTERN: OnceLock<Regex> = OnceLock::new();
    URL_PATTERN
        .get_or_init(|| Regex::new(r"(?i)#credential-").expect("valid credential anchor pattern"))
        .is_match(&super::publish_readiness::js_string_nullish(
            entry.get("url"),
        ))
        && LABEL_PATTERN
            .get_or_init(|| {
                Regex::new(r"(?i)(?-u:\b)major\s+in(?-u:\b)")
                    .expect("valid credential major label pattern")
            })
            .is_match(&super::publish_readiness::js_string_nullish(
                entry.get("label"),
            ))
}

pub fn is_catalog_credential_promotion_for_different_major(
    entry: &Value,
    generated_plans: &[Value],
) -> bool {
    if !is_catalog_credential_promotion_entry(entry) {
        return false;
    }
    let candidate_raw = extract_catalog_credential_major_name(entry.get("label"));
    let candidate_value = candidate_raw.map(Value::String);
    let candidate = normalize_catalog_credential_major_name(candidate_value.as_ref());
    let targets = promotion_plan_comparable_titles(entry, generated_plans);
    !candidate.is_empty() && !targets.is_empty() && !targets.contains(&candidate)
}

pub fn is_catalog_program_promotion_entry(entry: &Value) -> bool {
    static URL_PATTERN: OnceLock<Regex> = OnceLock::new();
    static LABEL_PATTERN: OnceLock<Regex> = OnceLock::new();
    URL_PATTERN
        .get_or_init(|| Regex::new(r"(?i)#program-").expect("valid program anchor pattern"))
        .is_match(&super::publish_readiness::js_string_nullish(
            entry.get("url"),
        ))
        && LABEL_PATTERN
            .get_or_init(|| {
                Regex::new(r"(?i)^Program of Study:\s*Major:")
                    .expect("valid program major label pattern")
            })
            .is_match(&super::publish_readiness::js_string_nullish(
                entry.get("label"),
            ))
}

pub fn is_catalog_program_promotion_for_different_major(
    entry: &Value,
    generated_plans: &[Value],
) -> bool {
    if !is_catalog_program_promotion_entry(entry) {
        return false;
    }
    let candidate_raw = extract_catalog_program_major_name(entry.get("label"));
    let candidate_value = candidate_raw.map(Value::String);
    let candidate = normalize_catalog_credential_major_name(candidate_value.as_ref());
    let targets = promotion_plan_comparable_titles(entry, generated_plans);
    !candidate.is_empty() && !targets.is_empty() && !targets.contains(&candidate)
}

fn is_likely_catalog_program_title_label(label: Option<&Value>) -> bool {
    static EXCLUSION_PATTERN: OnceLock<Regex> = OnceLock::new();
    let normalized = normalize_catalog_credential_major_name(label);
    let token_count = normalized.split_whitespace().count();
    !normalized.is_empty()
        && (2..=6).contains(&token_count)
        && !EXCLUSION_PATTERN
            .get_or_init(|| {
                Regex::new(r"(?i)(?-u:\b)(?:requirements?|options?|routes?|degree|scoped|section|home|program|credential)(?-u:\b)")
                    .expect("valid catalog title exclusion pattern")
            })
            .is_match(&super::publish_readiness::js_string_nullish(label))
}

pub fn is_catalog_program_page_promotion_for_different_major(
    entry: &Value,
    generated_plans: &[Value],
) -> bool {
    static URL_PATTERN: OnceLock<Regex> = OnceLock::new();
    if !URL_PATTERN
        .get_or_init(|| {
            Regex::new(r"(?i)/students/gencat/program/")
                .expect("valid catalog program page pattern")
        })
        .is_match(&super::publish_readiness::js_string_nullish(
            entry.get("url"),
        ))
        || !is_likely_catalog_program_title_label(entry.get("label"))
    {
        return false;
    }
    let candidate = normalize_catalog_credential_major_name(entry.get("label"));
    let targets = promotion_plan_comparable_titles(entry, generated_plans);
    !candidate.is_empty()
        && !targets.is_empty()
        && !targets.iter().any(|target| {
            target == &candidate || target.contains(&candidate) || candidate.contains(target)
        })
}

pub fn is_pathway_scoped_catalog_credential_promotion_for_broad_major_owner(
    entry: &Value,
    generated_plans: &[Value],
) -> bool {
    if entry.get("ownerType").and_then(Value::as_str) != Some("major")
        || entry.get("pathwayId").is_some_and(js_truthy)
        || !is_catalog_credential_promotion_entry(entry)
    {
        return false;
    }
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let plan_title = promotion_plan_title(entry, generated_plans);
    !plan_title.contains(':')
        && PATTERN
            .get_or_init(|| {
                Regex::new(r"(?i)(?-u:\b)major\s+in\s+[^:()]+:\s*\S")
                    .expect("valid broad-major child credential pattern")
            })
            .is_match(&super::publish_readiness::js_string_nullish(
                entry.get("label"),
            ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerivedOptionAliasTitle {
    pub parent_title: String,
    pub option_title: String,
}

fn normalize_alias_match_value(value: Option<&Value>) -> String {
    let text = super::publish_readiness::js_string_nullish(value);
    normalize_alias_match_text(&text)
}

pub fn parse_derived_option_alias_title(title: Option<&Value>) -> Option<DerivedOptionAliasTitle> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let title = super::publish_readiness::js_string_nullish(title);
    let title = trim_ecmascript_whitespace(&title);
    let captures = PATTERN
        .get_or_init(|| {
            Regex::new(r"^(.+?):\s*(.+?)(?:\s+(\([^)]+\)))?$")
                .expect("valid derived option alias title pattern")
        })
        .captures(title)?;
    let parent_base = captures
        .get(1)
        .map(|capture| trim_ecmascript_whitespace(capture.as_str()))
        .unwrap_or_default();
    let option_title = captures
        .get(2)
        .map(|capture| trim_ecmascript_whitespace(capture.as_str()))
        .unwrap_or_default();
    let credential_suffix = captures
        .get(3)
        .map(|capture| trim_ecmascript_whitespace(capture.as_str()))
        .unwrap_or_default();
    if parent_base.is_empty() || option_title.is_empty() || credential_suffix.is_empty() {
        return None;
    }
    Some(DerivedOptionAliasTitle {
        parent_title: format!("{parent_base} {credential_suffix}"),
        option_title: option_title.to_string(),
    })
}

fn pathway_matches_option_alias(pathway: &Value, option_title: &str) -> bool {
    let normalized_option_title =
        normalize_alias_match_value(Some(&Value::String(option_title.to_string())));
    let pathway_text = ["id", "label", "title"]
        .iter()
        .filter_map(|field| pathway.get(*field).filter(|value| js_truthy(value)))
        .map(super::publish_readiness::js_string)
        .collect::<Vec<_>>()
        .join(" ");
    let normalized_pathway_text = normalize_alias_match_value(Some(&Value::String(pathway_text)));
    if normalized_option_title.is_empty() || normalized_pathway_text.is_empty() {
        return false;
    }
    if normalized_pathway_text.contains(&normalized_option_title) {
        return true;
    }
    let option_tokens = normalized_option_title
        .split_whitespace()
        .filter(|token| !matches!(*token, "option" | "route" | "track" | "degree"))
        .collect::<Vec<_>>();
    !option_tokens.is_empty()
        && option_tokens
            .iter()
            .all(|token| normalized_pathway_text.contains(token))
}

/// Test the CJS validator's derived-parent-pathway coverage branch using runtime pathway records.
/// Each runtime record has `planId` and `pathways` fields, mirroring the generated runtime index.
pub fn has_derived_parent_pathway_runtime_coverage(
    owner: &Value,
    generated_plans: &[Value],
    runtime_pathway_records: &[Value],
) -> bool {
    if owner.get("ownerType").and_then(Value::as_str) != Some("major") {
        return false;
    }
    let Some(alias_title) = parse_derived_option_alias_title(owner.get("title")) else {
        return false;
    };
    let campus_id = trim_ecmascript_whitespace(&super::publish_readiness::js_string_nullish(
        owner.get("campusId"),
    ))
    .to_string();
    for parent_plan in generated_plans.iter().filter(|plan| {
        (campus_id.is_empty()
            || trim_ecmascript_whitespace(&super::publish_readiness::js_string_nullish(
                plan.get("campusId"),
            )) == campus_id)
            && normalize_alias_match_value(plan.get("title"))
                == normalize_alias_match_value(Some(&Value::String(
                    alias_title.parent_title.clone(),
                )))
    }) {
        let Some(parent_plan_id) = parent_plan.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(runtime_record) = runtime_pathway_records
            .iter()
            .rev()
            .find(|record| record.get("planId").and_then(Value::as_str) == Some(parent_plan_id))
        else {
            continue;
        };
        if value_array(runtime_record.get("pathways"))
            .iter()
            .any(|pathway| pathway_matches_option_alias(pathway, &alias_title.option_title))
        {
            return true;
        }
    }
    false
}

/// Check runtime alias coverage and the derived-parent-pathway fallback used by promotion selection.
/// `runtime_alias_covered_owner_ids` contains canonical owner IDs resolved from runtime alias data.
pub fn has_student_runtime_alias_coverage(
    owner: &Value,
    runtime_alias_covered_owner_ids: &HashSet<String>,
    generated_plans: &[Value],
    runtime_pathway_records: &[Value],
) -> bool {
    let Some(plan_id) = owner.get("planId").filter(|value| js_truthy(value)) else {
        return false;
    };
    let plan_id = super::publish_readiness::js_string(plan_id);
    let pathway_id = owner
        .get("pathwayId")
        .filter(|value| !value.is_null())
        .map(super::publish_readiness::js_string);
    let owner_id = super::source_fingerprints::build_transfer_planner_owner_id(
        Some(&plan_id),
        pathway_id.as_deref(),
    );
    runtime_alias_covered_owner_ids.contains(&owner_id)
        || has_derived_parent_pathway_runtime_coverage(
            owner,
            generated_plans,
            runtime_pathway_records,
        )
}

fn is_suspicious_structural_pathway_id(value: Option<&Value>) -> bool {
    let value = super::publish_readiness::js_string_nullish(value);
    super::source_pipeline_structural_patterns::is_suspicious_structural_pathway_id(&value)
}

pub fn is_suspicious_structural_pathway_promotion_entry(entry: &Value) -> bool {
    if entry.get("ownerType").and_then(Value::as_str) != Some("pathway") {
        return false;
    }
    let label = super::publish_readiness::js_string_nullish(entry.get("label"));
    let owner_title = super::publish_readiness::js_string_nullish(entry.get("ownerTitle"));
    is_suspicious_structural_pathway_id(entry.get("pathwayId"))
        || super::source_pipeline_structural_patterns::is_suspicious_structural_pathway_label(
            &label,
        )
        || super::source_pipeline_structural_patterns::is_suspicious_structural_pathway_label(
            &owner_title,
        )
}

/// Central promotion veto predicate. Structural pathway labels/IDs use the Rust materialization
/// heuristics; the boolean allows callers with the full materialization registry to add its result.
pub fn is_unsafe_automatic_promotion_entry(
    entry: &Value,
    generated_plans: &[Value],
    manual_overrides: &[Value],
    structural_pathway_suspicious: bool,
) -> bool {
    is_clearly_support_only_promotion_entry(entry)
        || is_minor_credential_promotion_for_major_owner(entry)
        || is_ba_bs_route_promotion_for_broad_major_owner(entry, generated_plans)
        || is_unscoped_acs_route_promotion_for_broad_major_owner(entry)
        || is_catalog_credential_promotion_for_different_major(entry, generated_plans)
        || is_catalog_program_promotion_for_different_major(entry, generated_plans)
        || is_catalog_program_page_promotion_for_different_major(entry, generated_plans)
        || is_pathway_scoped_catalog_credential_promotion_for_broad_major_owner(
            entry,
            generated_plans,
        )
        || is_suspicious_structural_pathway_promotion_entry(entry)
        || (entry.get("ownerType").and_then(Value::as_str) == Some("pathway")
            && structural_pathway_suspicious)
        || is_tacoma_global_studies_leaked_promotion_entry(entry)
        || is_seattle_jsis_asia_concentration_leaked_promotion_entry(entry)
        || is_skip_navigation_promotion_entry(entry)
        || should_skip_manual_override_promotion(
            entry.get("planId"),
            entry.get("pathwayId"),
            entry.get("url"),
            manual_overrides,
        )
        || has_document_url_with_appended_path(entry)
}

pub fn suggested_primary_is_safe_for_automatic_promotion(
    owner: &Value,
    generated_plans: &[Value],
    manual_overrides: &[Value],
    structural_pathway_suspicious: bool,
) -> bool {
    let candidate = build_promotion_entry_candidate_from_owner(owner);
    !is_unsafe_automatic_promotion_entry(
        &candidate,
        generated_plans,
        manual_overrides,
        structural_pathway_suspicious,
    )
}

pub fn get_missing_primary_auto_promotion_block_reason(
    owner: &Value,
    generated_plans: &[Value],
    manual_overrides: &[Value],
    structural_pathway_suspicious: bool,
) -> Option<&'static str> {
    if !is_schedulable_primary_suggestion(owner.get("suggestedPrimary")) {
        return Some("suggested primary is not a high-confidence schedulable primary");
    }
    if !suggested_primary_matches_specialized_plan_source(owner, generated_plans) {
        return Some("suggested primary does not match the specialized plan source");
    }
    if !suggested_primary_is_safe_for_automatic_promotion(
        owner,
        generated_plans,
        manual_overrides,
        structural_pathway_suspicious,
    ) {
        return Some("suggested primary is catalog-scoped to a different major or child option");
    }
    None
}

#[derive(Clone, Copy)]
pub struct SourcePipelinePromotionContext<'a> {
    pub generated_plans: &'a [Value],
    pub manual_overrides: &'a [Value],
    pub runtime_alias_covered_owner_ids: &'a HashSet<String>,
    pub runtime_pathway_records: &'a [Value],
    pub structurally_suspicious_owner_keys: &'a HashSet<String>,
}

impl SourcePipelinePromotionContext<'_> {
    fn has_runtime_alias_coverage(&self, owner: &Value) -> bool {
        has_student_runtime_alias_coverage(
            owner,
            self.runtime_alias_covered_owner_ids,
            self.generated_plans,
            self.runtime_pathway_records,
        )
    }

    fn is_suggestion_safe(&self, owner: &Value) -> bool {
        suggested_primary_is_safe_for_automatic_promotion(
            owner,
            self.generated_plans,
            self.manual_overrides,
            self.structurally_suspicious_owner_keys
                .contains(&build_owner_key(owner)),
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourcePipelineScopedOwnerSets {
    pub review_owner_keys: HashSet<String>,
    pub validation_scoped_discovery_owner_keys: HashSet<String>,
    pub validation_scoped_source_gap_discovery_owner_keys: HashSet<String>,
    pub validation_scoped_missing_primary_review_owner_keys: HashSet<String>,
    pub validation_scoped_review_owner_count: usize,
    pub source_gap_owner_keys: HashSet<String>,
}

pub fn build_source_pipeline_scoped_owner_sets(
    discovery_report: &Value,
    review_queue: &Value,
    source_gap_report: &Value,
    target_plan_id: Option<&str>,
    parser_backed_review_owner_keys: &HashSet<String>,
    active_pathways: &[Value],
    promotion_context: &SourcePipelinePromotionContext<'_>,
) -> SourcePipelineScopedOwnerSets {
    let discovery_owners =
        get_validation_scoped_discovery_owners(discovery_report, "owners", target_plan_id);
    let validation_scoped_discovery_owner_keys = discovery_owners
        .iter()
        .map(build_owner_key)
        .collect::<HashSet<_>>();
    let should_exclude_from_gaps = |owner: &Value| {
        let owner_key = build_owner_key(owner);
        parser_backed_review_owner_keys.contains(&owner_key)
            || is_structural_placeholder_review_owner(owner)
            || is_inactive_pathway_review_owner(owner, active_pathways)
            || promotion_context.has_runtime_alias_coverage(owner)
    };
    let validation_scoped_source_gap_discovery_owner_keys = discovery_owners
        .iter()
        .filter(|owner| !should_exclude_from_gaps(owner))
        .map(build_owner_key)
        .collect::<HashSet<_>>();
    let validation_scoped_review_entries =
        get_validation_scoped_review_queue_entries(review_queue, target_plan_id);
    let validation_scoped_missing_primary_review_owner_keys = validation_scoped_review_entries
        .iter()
        .filter(|entry| {
            let owner_key = build_owner_key(entry);
            validation_scoped_discovery_owner_keys.contains(&owner_key)
                && !should_exclude_from_gaps(entry)
        })
        .map(build_owner_key)
        .collect::<HashSet<_>>();
    let active_target = target_plan_id.filter(|plan_id| !plan_id.is_empty());
    let source_gap_owner_keys = value_array(source_gap_report.get("entries"))
        .iter()
        .filter(|entry| {
            active_target.is_none_or(|plan_id| {
                entry.get("planId") == Some(&Value::String(plan_id.to_string()))
            })
        })
        .map(build_owner_key)
        .collect::<HashSet<_>>();
    SourcePipelineScopedOwnerSets {
        review_owner_keys: build_review_owner_key_set(review_queue),
        validation_scoped_discovery_owner_keys,
        validation_scoped_source_gap_discovery_owner_keys,
        validation_scoped_missing_primary_review_owner_keys,
        validation_scoped_review_owner_count: validation_scoped_review_entries.len(),
        source_gap_owner_keys,
    }
}

fn promotion_result_entry(owner: &Value, blocked_reason: Option<&str>) -> Value {
    let candidate = build_promotion_entry_candidate_from_owner(owner);
    let mut result = serde_json::Map::new();
    if let Some(owner_id) = candidate.get("ownerId") {
        result.insert("ownerId".to_string(), owner_id.clone());
    }
    result.insert(
        "ownerKey".to_string(),
        Value::String(build_owner_key(owner)),
    );
    if let Some(title) = owner.get("title") {
        result.insert("title".to_string(), title.clone());
    }
    let promoted_url = owner
        .get("suggestedPrimary")
        .and_then(|suggested| suggested.get("url"));
    if let Some(blocked_reason) = blocked_reason {
        result.insert(
            "promotedUrl".to_string(),
            promoted_url
                .cloned()
                .filter(|url| !url.is_null())
                .unwrap_or(Value::Null),
        );
        result.insert(
            "blockReason".to_string(),
            Value::String(blocked_reason.to_string()),
        );
    } else if let Some(promoted_url) = promoted_url {
        result.insert("promotedUrl".to_string(), promoted_url.clone());
    }
    Value::Object(result)
}

pub fn get_eligible_missing_primary_auto_promotion_owners(
    discovery_report: &Value,
    review_queue: &Value,
    target_plan_id: Option<&str>,
    context: &SourcePipelinePromotionContext<'_>,
) -> Vec<Value> {
    let review_owner_keys = build_review_owner_key_set(review_queue);
    get_validation_scoped_discovery_owners(discovery_report, "owners", target_plan_id)
        .into_iter()
        .filter(|owner| !owner.get("existingPrimaryUrl").is_some_and(js_truthy))
        .filter(|owner| !context.has_runtime_alias_coverage(owner))
        .filter(|owner| is_schedulable_primary_suggestion(owner.get("suggestedPrimary")))
        .filter(|owner| {
            suggested_primary_matches_specialized_plan_source(owner, context.generated_plans)
        })
        .filter(|owner| context.is_suggestion_safe(owner))
        .filter(|owner| !review_owner_keys.contains(&build_owner_key(owner)))
        .map(|owner| promotion_result_entry(&owner, None))
        .collect()
}

pub fn get_blocked_missing_primary_auto_promotion_owners(
    discovery_report: &Value,
    review_queue: &Value,
    target_plan_id: Option<&str>,
    context: &SourcePipelinePromotionContext<'_>,
) -> Vec<Value> {
    let review_owner_keys = build_review_owner_key_set(review_queue);
    get_validation_scoped_discovery_owners(discovery_report, "owners", target_plan_id)
        .into_iter()
        .filter(|owner| !owner.get("existingPrimaryUrl").is_some_and(js_truthy))
        .filter(|owner| !context.has_runtime_alias_coverage(owner))
        .filter(|owner| !review_owner_keys.contains(&build_owner_key(owner)))
        .filter_map(|owner| {
            let blocked_reason = get_missing_primary_auto_promotion_block_reason(
                &owner,
                context.generated_plans,
                context.manual_overrides,
                context
                    .structurally_suspicious_owner_keys
                    .contains(&build_owner_key(&owner)),
            )?;
            Some(promotion_result_entry(&owner, Some(blocked_reason)))
        })
        .collect()
}

pub fn get_eligible_weak_existing_replacement_owners(
    discovery_report: &Value,
    review_queue: &Value,
    target_plan_id: Option<&str>,
    context: &SourcePipelinePromotionContext<'_>,
) -> Vec<Value> {
    let review_owner_keys = build_review_owner_key_set(review_queue);
    get_validation_scoped_discovery_owners(discovery_report, "weakExistingOwners", target_plan_id)
        .into_iter()
        .filter(|owner| {
            owner.get("suggestedAction").and_then(Value::as_str) == Some("replace-existing-primary")
        })
        .filter(|owner| !context.has_runtime_alias_coverage(owner))
        .filter(|owner| is_schedulable_primary_suggestion(owner.get("suggestedPrimary")))
        .filter(|owner| {
            suggested_primary_matches_specialized_plan_source(owner, context.generated_plans)
        })
        .filter(|owner| context.is_suggestion_safe(owner))
        .filter(|owner| {
            let owner_key = build_owner_key(owner);
            !review_owner_keys.contains(&owner_key)
                || current_primary_cannot_create_schedulable_rows(owner)
        })
        .map(|owner| promotion_result_entry(&owner, None))
        .collect()
}

pub fn get_eligible_auto_promotion_owners(
    discovery_report: &Value,
    review_queue: &Value,
    target_plan_id: Option<&str>,
    context: &SourcePipelinePromotionContext<'_>,
) -> Vec<Value> {
    let mut by_owner_id = HashMap::new();
    for owner in get_eligible_missing_primary_auto_promotion_owners(
        discovery_report,
        review_queue,
        target_plan_id,
        context,
    )
    .into_iter()
    .chain(get_eligible_weak_existing_replacement_owners(
        discovery_report,
        review_queue,
        target_plan_id,
        context,
    )) {
        by_owner_id.insert(js_property_string(owner.get("ownerId")), owner);
    }
    let mut owners = by_owner_id.into_values().collect::<Vec<_>>();
    owners.sort_by(|left, right| {
        locale_like_compare(
            &js_property_string(left.get("ownerId")),
            &js_property_string(right.get("ownerId")),
        )
    });
    owners
}

pub fn build_primary_manifest_owner_ids(manifest: &[Value]) -> HashSet<String> {
    manifest
        .iter()
        .filter(|entry| {
            matches!(
                entry.get("ownerType").and_then(Value::as_str),
                Some("major" | "pathway")
            ) && entry.get("campusId").is_some_and(js_truthy)
                && entry.get("campusId") != Some(&Value::String("grc".to_string()))
                && entry
                    .get("isPrimaryDegreeRequirementsLink")
                    .is_some_and(js_truthy)
        })
        .map(|entry| js_property_string(entry.get("ownerId")))
        .collect()
}

pub fn build_parseable_primary_manifest_owner_ids(manifest: &[Value]) -> HashSet<String> {
    const PARSEABLE_PARSER_TYPES: &[&str] = &[
        "html-degree-page",
        "html-curriculum-page",
        "html-overview-page",
        "catalog-page",
        "generic-html",
        "pdf-degree-sheet",
        "pdf-worksheet",
        "generic-pdf",
    ];
    manifest
        .iter()
        .filter(|entry| {
            matches!(
                entry.get("ownerType").and_then(Value::as_str),
                Some("major" | "pathway")
            ) && entry.get("campusId").is_some_and(js_truthy)
                && entry.get("campusId") != Some(&Value::String("grc".to_string()))
                && entry
                    .get("isPrimaryDegreeRequirementsLink")
                    .is_some_and(js_truthy)
                && entry
                    .get("parserType")
                    .and_then(Value::as_str)
                    .is_some_and(|parser_type| PARSEABLE_PARSER_TYPES.contains(&parser_type))
        })
        .map(|entry| js_property_string(entry.get("ownerId")))
        .collect()
}

pub fn build_owner_key(owner: &Value) -> String {
    let owner_id = owner
        .get("ownerKey")
        .filter(|value| !value.is_null())
        .or_else(|| owner.get("ownerId").filter(|value| !value.is_null()));
    normalized_owner_key(owner_id, owner.get("planId"), owner.get("pathwayId"))
}

pub fn get_review_queue_entries(review_queue: &Value) -> Vec<Value> {
    review_queue
        .get("campuses")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|campus| campus.get("entries").and_then(Value::as_array))
        .flatten()
        .cloned()
        .collect()
}

pub fn build_review_owner_key_set(review_queue: &Value) -> HashSet<String> {
    get_review_queue_entries(review_queue)
        .iter()
        .map(build_owner_key)
        .collect()
}

pub fn get_validation_scoped_review_queue_entries(
    review_queue: &Value,
    target_plan_id: Option<&str>,
) -> Vec<Value> {
    let entries = get_review_queue_entries(review_queue);
    let Some(target_plan_id) = target_plan_id.filter(|plan_id| !plan_id.is_empty()) else {
        return entries;
    };
    entries
        .into_iter()
        .filter(|entry| entry.get("planId") == Some(&Value::String(target_plan_id.to_string())))
        .collect()
}

pub fn get_validation_scoped_discovery_owners(
    discovery_report: &Value,
    field_name: &str,
    target_plan_id: Option<&str>,
) -> Vec<Value> {
    let owners = discovery_report
        .get(field_name)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let Some(target_plan_id) = target_plan_id.filter(|plan_id| !plan_id.is_empty()) else {
        return owners;
    };
    owners
        .into_iter()
        .filter(|owner| owner.get("planId") == Some(&Value::String(target_plan_id.to_string())))
        .collect()
}

pub fn compare_string_sets(left: &[String], right: &[String]) -> (Vec<String>, Vec<String>) {
    let left_set = left.iter().cloned().collect::<HashSet<_>>();
    let right_set = right.iter().cloned().collect::<HashSet<_>>();
    let mut left_only = left_set.difference(&right_set).cloned().collect::<Vec<_>>();
    let mut right_only = right_set.difference(&left_set).cloned().collect::<Vec<_>>();
    left_only.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
    right_only.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
    (left_only, right_only)
}

pub fn get_validation_scoped_discovery_owner_count(
    discovery_report: &Value,
    target_plan_id: Option<&str>,
) -> usize {
    get_validation_scoped_discovery_owners(discovery_report, "owners", target_plan_id).len()
}

pub fn is_structural_placeholder_review_owner(owner: &Value) -> bool {
    let pathway_id_value = super::publish_readiness::js_string_nullish(owner.get("pathwayId"));
    let pathway_id = trim_ecmascript_whitespace(&pathway_id_value);
    let owner_key = super::publish_readiness::js_string_nullish(owner.get("ownerKey"));
    pathway_id == "four-option"
        || owner_key
            .to_ascii_lowercase()
            .ends_with(":pathway:four-option")
}

pub fn is_inactive_pathway_review_owner(owner: &Value, active_pathways: &[Value]) -> bool {
    if owner.get("ownerType").and_then(Value::as_str) != Some("pathway") {
        return false;
    }
    let owner_key = build_owner_key(owner);
    !active_pathways.iter().any(|pathway| {
        normalized_owner_key(
            pathway.get("id"),
            pathway.get("planId"),
            pathway.get("pathwayId"),
        ) == owner_key
    })
}

pub fn is_schedulable_primary_suggestion(candidate: Option<&Value>) -> bool {
    let Some(candidate) = candidate else {
        return false;
    };
    candidate.get("confidence").and_then(Value::as_str) == Some("high")
        && candidate.get("canCreateSchedulableRows") != Some(&Value::Bool(false))
        && candidate.get("sourceRoleStatus").and_then(Value::as_str) != Some("support")
        && candidate.get("sourceRole").and_then(Value::as_str) != Some("support-source")
        && candidate.get("sourceRole").and_then(Value::as_str) != Some("curriculum-map")
        && candidate.get("parserType").and_then(Value::as_str) != Some("html-curriculum-page")
}

pub fn current_primary_cannot_create_schedulable_rows(owner: &Value) -> bool {
    let Some(current_primary) = owner.get("currentPrimary").filter(|value| js_truthy(value)) else {
        return false;
    };
    current_primary
        .get("sourceRoleStatus")
        .and_then(Value::as_str)
        != Some("primary")
        || current_primary.get("canCreateSchedulableRows") == Some(&Value::Bool(false))
        || current_primary.get("canBePrimary") == Some(&Value::Bool(false))
}

/// Match parsed requirement blocks to active pathways for the validator's parser-backed exclusions.
pub fn build_parser_backed_review_owner_keys(
    parsed_blocks: &[Value],
    active_pathways: &[Value],
) -> HashSet<String> {
    let active_owner_keys = active_pathways
        .iter()
        .map(|pathway| {
            normalized_owner_key(
                pathway.get("id"),
                pathway.get("planId"),
                pathway.get("pathwayId"),
            )
        })
        .collect::<HashSet<_>>();
    let mut pathways_by_plan = std::collections::HashMap::<String, Vec<&Value>>::new();
    for pathway in active_pathways {
        let plan_id_value = super::publish_readiness::js_string_nullish(pathway.get("planId"));
        let plan_id = trim_ecmascript_whitespace(&plan_id_value);
        if !plan_id.is_empty() {
            pathways_by_plan
                .entry(plan_id.to_string())
                .or_default()
                .push(pathway);
        }
    }

    let mut keys = HashSet::new();
    for block in parsed_blocks {
        let plan_id_value = block.get("planId");
        if !js_truthy(block.get("ok").unwrap_or(&Value::Null))
            || !plan_id_value.is_some_and(js_truthy)
            || !can_parsed_block_create_required_schedule_rows(block)
        {
            continue;
        }
        let plan_id = super::publish_readiness::js_string_nullish(plan_id_value);
        keys.insert(normalized_owner_key(
            Some(&Value::String(plan_id.clone())),
            Some(&Value::String(plan_id.clone())),
            None,
        ));

        if let Some(pathway_id) = block.get("pathwayId").filter(|value| js_truthy(value)) {
            let owner_id = block
                .get("ownerId")
                .filter(|value| js_truthy(value))
                .map(super::publish_readiness::js_string)
                .unwrap_or_else(|| {
                    super::source_fingerprints::build_transfer_planner_owner_id(
                        Some(&plan_id),
                        Some(&super::publish_readiness::js_string(pathway_id)),
                    )
                });
            keys.insert(normalized_owner_key(
                Some(&Value::String(owner_id)),
                Some(&Value::String(plan_id.clone())),
                Some(pathway_id),
            ));
        }
        keys.extend(parser_backed_review_owner_keys_from_labels(
            block,
            &plan_id,
            &active_owner_keys,
        ));
        keys.extend(parser_backed_review_owner_keys_from_source_text(
            block,
            &plan_id,
            &pathways_by_plan,
            &active_owner_keys,
        ));
    }
    keys
}

fn can_parsed_block_create_required_schedule_rows(block: &Value) -> bool {
    for field in [
        "canCreateScheduleRows",
        "canCreateRequiredRows",
        "canCreateSchedulableRows",
    ] {
        if block.get(field) == Some(&Value::Bool(false)) {
            return false;
        }
    }
    for field in ["supportOnly", "nonSchedulable"] {
        if block.get(field) == Some(&Value::Bool(true)) {
            return false;
        }
    }

    let status = super::publish_readiness::js_string_nullish(block.get("sourceRoleStatus"));
    if ["support", "non-schedulable", "ignored"].contains(&status.as_str()) {
        return false;
    }
    let role = super::publish_readiness::js_string_nullish(block.get("sourceRole"));
    ![
        "approved-course-list",
        "elective-list",
        "upper-division-prerequisite-table",
        "non-schedulable-course-list",
        "sample-schedule",
        "support-source",
        "admission-prerequisite-source",
        "admissions-preparation",
        "transfer-equivalency",
        "matched-grc-track",
        "old-archival",
        "ignored",
    ]
    .contains(&role.as_str())
}

fn normalized_owner_key(
    owner_id: Option<&Value>,
    plan_id: Option<&Value>,
    pathway_id: Option<&Value>,
) -> String {
    let owner_id = owner_id.map_or_else(String::new, super::publish_readiness::js_string);
    let plan_id = plan_id
        .filter(|value| !value.is_null())
        .map(super::publish_readiness::js_string);
    let pathway_id = pathway_id
        .filter(|value| !value.is_null())
        .map(super::publish_readiness::js_string);
    super::source_fingerprints::normalize_transfer_planner_owner_id(
        &owner_id,
        plan_id.as_deref(),
        pathway_id.as_deref(),
    )
}

fn slugify_pathway_id(value: &Value) -> String {
    let text = trim_ecmascript_whitespace(&super::publish_readiness::js_string(value))
        .to_lowercase()
        .replace('&', " and ");
    let mut slug = String::new();
    let mut pending_dash = false;
    for character in text.chars() {
        if character.is_ascii_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(character);
        } else {
            pending_dash = !slug.is_empty();
        }
    }
    slug
}

fn parser_backed_review_owner_keys_from_labels(
    block: &Value,
    plan_id: &str,
    active_owner_keys: &HashSet<String>,
) -> HashSet<String> {
    let Some(labels) = block.get("pathwayLabels").and_then(Value::as_array) else {
        return HashSet::new();
    };
    labels
        .iter()
        .filter_map(|label| {
            let slug = slugify_pathway_id(label);
            let pathway_id = super::source_fingerprints::normalize_transfer_planner_pathway_id(
                Some(plan_id),
                Some(&slug),
            )?;
            let owner_id = super::source_fingerprints::build_transfer_planner_owner_id(
                Some(plan_id),
                Some(&pathway_id),
            );
            let owner_key = normalized_owner_key(
                Some(&Value::String(owner_id)),
                Some(&Value::String(plan_id.to_string())),
                Some(&Value::String(pathway_id)),
            );
            active_owner_keys.contains(&owner_key).then_some(owner_key)
        })
        .collect()
}

fn parser_backed_review_owner_keys_from_source_text(
    block: &Value,
    plan_id: &str,
    pathways_by_plan: &std::collections::HashMap<String, Vec<&Value>>,
    active_owner_keys: &HashSet<String>,
) -> HashSet<String> {
    if block.get("pathwayId").is_some_and(js_truthy) {
        return HashSet::new();
    }
    pathways_by_plan
        .get(plan_id)
        .into_iter()
        .flatten()
        .filter(|pathway| parser_backed_block_mentions_pathway(block, pathway))
        .filter_map(|pathway| {
            let owner_key = normalized_owner_key(
                pathway.get("id"),
                pathway.get("planId"),
                pathway.get("pathwayId"),
            );
            active_owner_keys.contains(&owner_key).then_some(owner_key)
        })
        .collect()
}

fn parser_backed_block_mentions_pathway(block: &Value, pathway: &Value) -> bool {
    let pathway_id_tokens = pathway
        .get("pathwayId")
        .map(|value| parser_backed_pathway_tokens([super::publish_readiness::js_string(value)]))
        .unwrap_or_default();
    let fallback_tokens = if pathway_id_tokens.is_empty() {
        pathway
            .get("label")
            .map(|value| parser_backed_pathway_tokens([super::publish_readiness::js_string(value)]))
            .unwrap_or_default()
    } else {
        pathway_id_tokens
    };
    if fallback_tokens.is_empty() {
        return false;
    }

    let mut evidence_values = [
        "ownerTitle",
        "sourceLabel",
        "primarySourceLabel",
        "sourceUrl",
        "primarySourceUrl",
    ]
    .into_iter()
    .filter_map(|field| block.get(field).map(super::publish_readiness::js_string))
    .collect::<Vec<_>>();
    for field in ["pathwayLabels", "requirementCueLines", "chooseStatements"] {
        if let Some(values) = block.get(field) {
            match values {
                Value::Array(values) => {
                    evidence_values.extend(values.iter().map(super::publish_readiness::js_string))
                }
                Value::String(value) => {
                    evidence_values.extend(value.chars().map(|character| character.to_string()))
                }
                _ => {}
            }
        }
    }
    let evidence_tokens = parser_backed_pathway_tokens(evidence_values);
    fallback_tokens
        .iter()
        .all(|token| evidence_tokens.contains(token))
}

fn parser_backed_pathway_tokens(values: impl IntoIterator<Item = String>) -> HashSet<String> {
    let mut tokens = HashSet::new();
    for value in values {
        for token in normalize_alias_match_text(&value).split_whitespace() {
            let normalized = normalize_parser_backed_pathway_token(token);
            if normalized.len() >= 3 && !is_parser_backed_pathway_stopword(&normalized) {
                tokens.insert(normalized);
            }
        }
    }
    tokens
}

fn normalize_alias_match_text(value: &str) -> String {
    static PARENTHETICAL: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let without_parenthetical = PARENTHETICAL
        .get_or_init(|| Regex::new(r"\([^)]*\)").expect("parenthetical regex is valid"))
        .replace_all(value, " ");
    let mut output = String::new();
    let mut pending_space = false;
    for character in without_parenthetical
        .to_lowercase()
        .replace('&', " and ")
        .chars()
    {
        if character.is_ascii_alphanumeric() {
            if pending_space && !output.is_empty() {
                output.push(' ');
            }
            pending_space = false;
            output.push(character);
        } else {
            pending_space = !output.is_empty();
        }
    }
    output
}

fn normalize_parser_backed_pathway_token(token: &str) -> String {
    if token.len() > 4 && token.ends_with("ies") {
        format!("{}y", &token[..token.len() - 3])
    } else if token.len() > 4 && token.ends_with('s') {
        token[..token.len() - 1].to_string()
    } else {
        token.to_string()
    }
}

fn is_parser_backed_pathway_stopword(token: &str) -> bool {
    matches!(
        token,
        "and"
            | "ba"
            | "bachelor"
            | "bs"
            | "degree"
            | "major"
            | "of"
            | "option"
            | "pathway"
            | "program"
            | "route"
            | "the"
            | "track"
            | "uw"
            | "washington"
    )
}

/// Return canonical parseable manifest sources without a successful owner- or plan/source-level
/// parser result. The plan/source fallback preserves duplicate pathway aliases in CJS behavior.
pub fn find_failed_parseable_requirement_sources(
    entries: &[Value],
    owners: &[Value],
) -> Vec<Value> {
    let mut successful_source_keys = HashSet::new();
    let mut successful_plan_source_keys = HashSet::new();
    for owner in owners
        .iter()
        .filter(|owner| owner.get("ok") == Some(&Value::Bool(true)))
    {
        successful_source_keys.extend(build_report_parse_keys(owner));
        successful_plan_source_keys.extend(build_report_plan_source_keys(owner));
    }

    entries
        .iter()
        .filter(|entry| {
            !successful_source_keys.contains(&build_manifest_parse_key(entry))
                && !successful_plan_source_keys.contains(&build_manifest_plan_source_key(entry))
        })
        .cloned()
        .collect()
}

fn build_manifest_parse_key(entry: &Value) -> String {
    format!(
        "{}::{}",
        js_property_string(entry.get("ownerId")),
        normalize_requirement_source_url_for_coverage(entry.get("url"))
    )
}

fn build_manifest_plan_source_key(entry: &Value) -> String {
    let plan_id = entry
        .get("planId")
        .filter(|value| !value.is_null())
        .or_else(|| entry.get("ownerId"));
    format!(
        "{}::{}",
        js_property_string(plan_id),
        normalize_requirement_source_url_for_coverage(entry.get("url"))
    )
}

fn build_report_parse_keys(owner: &Value) -> Vec<String> {
    let owner_id = js_property_string(owner.get("ownerId"));
    let mut keys = Vec::new();
    for source_url in report_source_urls(owner) {
        if js_truthy(&source_url) {
            keys.push(format!(
                "{owner_id}::{}",
                normalize_requirement_source_url_for_coverage(Some(&source_url))
            ));
        }
    }
    unique_locale_sorted(keys)
}

fn build_report_plan_source_keys(owner: &Value) -> Vec<String> {
    let owner_id = owner.get("ownerId");
    let (plan_id, plan_id_is_truthy) =
        if let Some(plan_id) = owner.get("planId").filter(|value| !value.is_null()) {
            (
                super::publish_readiness::js_string(plan_id),
                js_truthy(plan_id),
            )
        } else if let Some(owner_id) = owner_id.filter(|value| !value.is_null()) {
            if let Some(owner_id) = owner_id.as_str() {
                let plan_id = owner_id.split(":pathway:").next().unwrap_or(owner_id);
                (plan_id.to_string(), !plan_id.is_empty())
            } else {
                (
                    super::publish_readiness::js_string(owner_id),
                    js_truthy(owner_id),
                )
            }
        } else {
            return Vec::new();
        };
    if !plan_id_is_truthy {
        return Vec::new();
    }

    let mut keys = Vec::new();
    for source_url in report_source_urls(owner) {
        if js_truthy(&source_url) {
            keys.push(format!(
                "{plan_id}::{}",
                normalize_requirement_source_url_for_coverage(Some(&source_url))
            ));
        }
    }
    unique_locale_sorted(keys)
}

fn report_source_urls(owner: &Value) -> Vec<Value> {
    let mut urls = [owner.get("primarySourceUrl"), owner.get("sourceUrl")]
        .into_iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    match owner.get("coveredSourceUrls") {
        Some(Value::Array(covered)) => urls.extend(covered.iter().cloned()),
        Some(Value::String(covered)) => urls.extend(
            covered
                .chars()
                .map(|character| Value::String(character.to_string())),
        ),
        _ => {}
    }
    urls
}

fn unique_locale_sorted(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut unique = values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect::<Vec<_>>();
    unique.sort_by(|left, right| locale_like_compare(left, right));
    unique
}

fn locale_like_compare(left: &str, right: &str) -> Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| left.cmp(right))
}

fn js_property_string(value: Option<&Value>) -> String {
    value.map_or_else(
        || "undefined".to_string(),
        super::publish_readiness::js_string,
    )
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

fn trim_ecmascript_whitespace(value: &str) -> &str {
    let start = value
        .char_indices()
        .find_map(|(index, character)| (!is_ecmascript_whitespace(character)).then_some(index));
    let Some(start) = start else {
        return "";
    };
    let end = value
        .char_indices()
        .rev()
        .find_map(|(index, character)| {
            (!is_ecmascript_whitespace(character)).then_some(index + character.len_utf8())
        })
        .unwrap_or(start);
    &value[start..end]
}

fn is_ecmascript_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'..='\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'..='\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn report_metrics_deduplicate_and_sort_owner_sets_with_nullish_fallbacks() {
        let eligible = vec![
            json!({"ownerId": "plan-b", "ownerKey": "key-b"}),
            json!({"ownerId": "plan-a", "ownerKey": "key-a"}),
            json!({"ownerId": "plan-a", "ownerKey": "key-a"}),
        ];
        let discovery = json!({
            "weakExistingOwners": [
                {"suggestedAction": "replace-existing-primary"},
                {"suggestedAction": "keep-existing-primary"}
            ],
            "highConfidenceReplacementCount": null
        });
        let review = HashSet::from(["review-1".to_string(), "review-2".to_string()]);
        let gaps = HashSet::from(["gap-1".to_string()]);
        let promotions = HashSet::from(["promoted-1".to_string(), "promoted-2".to_string()]);
        let promotion_keys = HashSet::from(["z-key".to_string(), "a-key".to_string()]);
        let primary = HashSet::from(["primary-1".to_string()]);
        let parseable = HashSet::from(["parseable-1".to_string(), "parseable-2".to_string()]);
        let parsed = HashSet::from(["parsed-1".to_string()]);
        let fingerprints = HashSet::from(["fingerprint-1".to_string()]);
        let metrics =
            build_source_pipeline_validation_metrics(SourcePipelineValidationMetricInputs {
                validation_target_plan_id: Some("target-plan"),
                eligible_auto_promotion_owners: &eligible,
                discovery_report: &discovery,
                review_owner_keys: &review,
                source_gap_owner_keys: &gaps,
                promoted_owner_ids: &promotions,
                promoted_owner_keys: &promotion_keys,
                primary_manifest_owner_ids: &primary,
                parseable_primary_owner_ids: &parseable,
                parsed_owner_ids: &parsed,
                requirement_fingerprint_owner_ids: &fingerprints,
            });
        assert_eq!(
            metrics.validation_target_plan_id.as_deref(),
            Some("target-plan")
        );
        assert_eq!(metrics.eligible_auto_promotion_owner_count, 2);
        assert_eq!(
            metrics.eligible_auto_promotion_owner_ids,
            ["plan-a", "plan-b"]
        );
        assert_eq!(
            metrics.eligible_auto_promotion_owner_keys,
            ["key-a", "key-b"]
        );
        assert_eq!(metrics.weak_existing_owner_count, 2);
        assert_eq!(metrics.high_confidence_replacement_owner_count, 1);
        assert_eq!(metrics.review_queue_owner_count, 2);
        assert_eq!(metrics.source_gap_owner_count, 1);
        assert_eq!(metrics.promoted_owner_count, 2);
        assert_eq!(metrics.promoted_owner_keys, ["a-key", "z-key"]);
        assert_eq!(metrics.canonical_primary_owner_count, 1);
        assert_eq!(metrics.parseable_primary_owner_count, 2);
        assert_eq!(metrics.parsed_owner_count, 1);
        assert_eq!(metrics.requirement_fingerprint_owner_count, 1);

        let explicit_zero = json!({"weakExistingOwners": [{"suggestedAction": "replace-existing-primary"}], "highConfidenceReplacementCount": 0});
        let metrics_with_explicit_zero =
            build_source_pipeline_validation_metrics(SourcePipelineValidationMetricInputs {
                validation_target_plan_id: None,
                eligible_auto_promotion_owners: &[],
                discovery_report: &explicit_zero,
                review_owner_keys: &HashSet::new(),
                source_gap_owner_keys: &HashSet::new(),
                promoted_owner_ids: &HashSet::new(),
                promoted_owner_keys: &HashSet::new(),
                primary_manifest_owner_ids: &HashSet::new(),
                parseable_primary_owner_ids: &HashSet::new(),
                parsed_owner_ids: &HashSet::new(),
                requirement_fingerprint_owner_ids: &HashSet::new(),
            });
        assert_eq!(
            metrics_with_explicit_zero.high_confidence_replacement_owner_count,
            0
        );
    }

    #[test]
    fn scoped_owner_set_builder_matches_discovery_review_gap_and_alias_filters() {
        let discovery = json!({"owners": [
            {"ownerType": "major", "ownerKey": "target", "planId": "target"},
            {"ownerType": "pathway", "ownerKey": "target:pathway:parsed", "planId": "target", "pathwayId": "parsed"},
            {"ownerType": "pathway", "ownerKey": "target:pathway:four-option", "planId": "target", "pathwayId": "four-option"},
            {"ownerType": "pathway", "ownerKey": "target:pathway:inactive-route", "planId": "target", "pathwayId": "inactive-route"},
            {"ownerType": "pathway", "ownerKey": "target:pathway:alias-route", "planId": "target", "pathwayId": "alias-route"},
            {"ownerType": "pathway", "ownerKey": "target:pathway:active-route", "planId": "target", "pathwayId": "active-route"},
            {"ownerType": "major", "ownerKey": "other", "planId": "other"}
        ]});
        let review_entries = [
            ("target", "target", "major", ""),
            ("target:pathway:parsed", "target", "pathway", "parsed"),
            (
                "target:pathway:four-option",
                "target",
                "pathway",
                "four-option",
            ),
            (
                "target:pathway:inactive-route",
                "target",
                "pathway",
                "inactive-route",
            ),
            (
                "target:pathway:alias-route",
                "target",
                "pathway",
                "alias-route",
            ),
            (
                "target:pathway:active-route",
                "target",
                "pathway",
                "active-route",
            ),
            ("other", "other", "major", ""),
        ];
        let mut review_entries = review_entries
            .iter()
            .map(|(owner_key, plan_id, owner_type, pathway_id)| {
                let mut entry = json!({
                    "ownerKey": owner_key,
                    "planId": plan_id,
                    "ownerType": owner_type
                });
                if !pathway_id.is_empty() {
                    entry["pathwayId"] = json!(pathway_id);
                }
                entry
            })
            .collect::<Vec<_>>();
        review_entries.push(review_entries[0].clone());
        let review = json!({"campuses": [{"entries": review_entries}]});
        let gaps = json!({"entries": [
            {"ownerType": "major", "ownerKey": "target", "planId": "target"},
            {"ownerType": "pathway", "ownerKey": "target:pathway:alias-route", "planId": "target", "pathwayId": "alias-route"},
            {"ownerType": "major", "ownerKey": "other", "planId": "other"}
        ]});
        let parser_backed = HashSet::from(["target:pathway:parsed".to_string()]);
        let active_pathways = [json!({
            "id": "active-route",
            "planId": "target",
            "pathwayId": "active-route"
        })];
        let generated_plans = [];
        let manual_overrides = [];
        let alias_covered = HashSet::from(["target:pathway:alias-route".to_string()]);
        let runtime_pathway_records = [];
        let suspicious_keys = HashSet::new();
        let promotion_context = SourcePipelinePromotionContext {
            generated_plans: &generated_plans,
            manual_overrides: &manual_overrides,
            runtime_alias_covered_owner_ids: &alias_covered,
            runtime_pathway_records: &runtime_pathway_records,
            structurally_suspicious_owner_keys: &suspicious_keys,
        };
        let scoped = build_source_pipeline_scoped_owner_sets(
            &discovery,
            &review,
            &gaps,
            Some("target"),
            &parser_backed,
            &active_pathways,
            &promotion_context,
        );
        assert_eq!(scoped.review_owner_keys.len(), 7);
        assert_eq!(scoped.validation_scoped_discovery_owner_keys.len(), 6);
        assert_eq!(
            scoped.validation_scoped_source_gap_discovery_owner_keys,
            HashSet::from([
                "target".to_string(),
                "target:pathway:active-route".to_string(),
            ])
        );
        assert_eq!(
            scoped.validation_scoped_missing_primary_review_owner_keys,
            scoped.validation_scoped_source_gap_discovery_owner_keys
        );
        assert_eq!(scoped.validation_scoped_review_owner_count, 7);
        assert_eq!(
            scoped.source_gap_owner_keys,
            HashSet::from([
                "target".to_string(),
                "target:pathway:alias-route".to_string(),
            ])
        );
    }

    #[test]
    fn review_and_promotion_checks_enforce_registry_queue_and_gap_consistency() {
        let promotion = json!({"ownerId": "plan", "ownerKey": "plan"});
        let primary_owners = HashSet::from(["plan".to_string()]);
        let discovery = json!({"owners": [{
            "ownerId": "plan",
            "currentPrimary": {"sourceRoleStatus": "primary", "canCreateSchedulableRows": true}
        }]});
        let aligned = build_review_and_promotion_checks(
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            std::slice::from_ref(&promotion),
            &primary_owners,
            &discovery,
        );
        assert_eq!(aligned.review_queue_alignment_check.status, "passed");
        assert_eq!(aligned.promotion_materialization_check.status, "passed");

        let mismatch = build_review_and_promotion_checks(
            &HashSet::from(["review-only".to_string()]),
            &HashSet::from(["plan".to_string()]),
            &HashSet::from(["plan".to_string()]),
            &[promotion.clone()],
            &primary_owners,
            &discovery,
        );
        assert_eq!(mismatch.review_queue_alignment_check.status, "failed");
        assert!(
            mismatch.promotion_materialization_check.details[0]
                .contains("remain in the review queue")
        );

        let missing_registry = build_review_and_promotion_checks(
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &[promotion],
            &HashSet::new(),
            &discovery,
        );
        assert!(
            missing_registry.promotion_materialization_check.details[0]
                .contains("missing canonical primary entries")
        );
    }

    #[test]
    fn discovery_partition_counts_only_scoped_missing_primary_owners() {
        let scoped = HashSet::from(["a".to_string(), "b".to_string(), "c".to_string()]);
        let eligible = [json!({"ownerId": "a"}), json!({"ownerId": "outside"})];
        let blocked = [json!({"ownerId": "c"})];
        let review = HashSet::from(["b".to_string()]);
        let check = build_discovery_partition_check(
            &scoped,
            &eligible,
            &blocked,
            &review,
            &[json!({"ownerId": "weak"})],
            4,
        );
        assert_eq!(check.status, "passed");
        assert_eq!(check.details[0], "Discovery owners: 3");
        assert_eq!(
            check.details[1],
            "Eligible missing-primary auto-promotions: 1"
        );
        assert_eq!(
            check.details[2],
            "Blocked missing-primary auto-promotions: 1"
        );

        let mismatch = build_discovery_partition_check(&scoped, &eligible, &[], &review, &[], 4);
        assert_eq!(mismatch.status, "failed");
        assert!(mismatch.details[0].contains("Discovery owner count should equal"));
    }

    #[test]
    fn eligible_promotion_checks_honor_target_scope_and_clearance_sets() {
        let owner = json!({"ownerId": "plan", "ownerKey": "plan"});
        let no_target = build_eligible_promotion_checks(
            None,
            std::slice::from_ref(&owner),
            &HashSet::from(["plan".to_string()]),
            &HashSet::new(),
            &HashSet::new(),
        );
        assert_eq!(
            no_target.eligible_discoveries_promoted_check.status,
            "passed"
        );
        assert_eq!(no_target.eligible_promotions_cleared_check.status, "passed");

        let target = build_eligible_promotion_checks(
            Some("plan"),
            std::slice::from_ref(&owner),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        assert_eq!(target.eligible_discoveries_promoted_check.status, "passed");
        assert!(target.eligible_discoveries_promoted_check.details[0].contains("Skipped"));

        let lingering = build_eligible_promotion_checks(
            None,
            &[owner],
            &HashSet::new(),
            &HashSet::from(["plan".to_string()]),
            &HashSet::from(["plan".to_string()]),
        );
        assert_eq!(
            lingering.eligible_discoveries_promoted_check.status,
            "failed"
        );
        assert!(
            lingering.eligible_promotions_cleared_check.details[0]
                .contains("still in review queue")
        );
    }

    #[test]
    fn artifact_alignment_checks_and_repairs_match_generated_report_relationships() {
        let promotion = json!({
            "ownerId": "plan",
            "planId": "plan",
            "url": "https://example.edu/requirements"
        });
        let parse_owner = json!({
            "ownerId": "plan",
            "planId": "plan",
            "ok": true,
            "primarySourceUrl": "https://example.edu/requirements"
        });
        let aligned = build_artifact_alignment_checks(
            &json!({"totalPromotions": 1}),
            std::slice::from_ref(&promotion),
            std::slice::from_ref(&promotion),
            &json!({"totalOwners": 1, "owners": [parse_owner.clone()]}),
            &json!({"totalRequirementSourceFingerprints": 1}),
            &[json!({"ownerId": "plan"})],
        );
        assert!(
            [
                &aligned.promotion_report_check,
                &aligned.promoted_owners_check,
                &aligned.canonical_parse_check,
                &aligned.fingerprint_alignment_check,
            ]
            .iter()
            .all(|check| check.status == "passed")
        );
        assert_eq!(aligned.promoted_owner_coverage_repair["needed"], false);
        assert_eq!(aligned.fingerprint_alignment_repair["needed"], false);

        let missing = build_artifact_alignment_checks(
            &json!({"totalPromotions": 1}),
            &[promotion.clone()],
            &[promotion],
            &json!({"totalOwners": 0, "owners": []}),
            &json!({"totalRequirementSourceFingerprints": 0}),
            &[],
        );
        assert_eq!(missing.promoted_owners_check.status, "failed");
        assert_eq!(missing.canonical_parse_check.status, "failed");
        assert_eq!(missing.promoted_owner_coverage_repair["needed"], true);
        assert_eq!(
            missing.promoted_owner_coverage_repair["targetPlanIds"],
            json!(["plan"])
        );
        assert_eq!(missing.fingerprint_alignment_repair["needed"], false);
    }

    #[test]
    fn registry_parser_alignment_accepts_plan_source_aliases_and_proposes_missing_coverage() {
        let entry = json!({
            "ownerId": "plan:pathway:alias-one",
            "planId": "plan",
            "pathwayId": "alias-one",
            "ownerType": "pathway",
            "url": "https://example.edu/requirements",
            "isPrimaryDegreeRequirementsLink": true
        });
        let parser_owner = json!({
            "ownerId": "plan:pathway:alias-two",
            "planId": "plan",
            "ok": true,
            "sourceUrl": "https://example.edu/requirements"
        });
        let aligned = build_registry_parser_alignment_result(
            std::slice::from_ref(&entry),
            &json!({"totalOwners": 1, "owners": [parser_owner.clone()]}),
            std::slice::from_ref(&parser_owner),
            1,
        );
        assert_eq!(aligned.check.status, "passed");
        assert_eq!(aligned.canonical_parseable_source_key_count, 1);
        assert_eq!(aligned.parser_output_left_only_count, 1);
        assert_eq!(aligned.parser_output_right_only_count, 1);
        assert_eq!(aligned.repair["needed"], false);

        let missing = build_registry_parser_alignment_result(
            &[entry],
            &json!({"totalOwners": 0, "owners": []}),
            &[],
            1,
        );
        assert_eq!(missing.check.status, "failed");
        assert_eq!(missing.repair["needed"], true);
        assert_eq!(missing.repair["targetPlanIds"], json!(["plan"]));
        assert_eq!(
            missing.repair["missingCurrentSourceCoverage"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn async_check_runner_matches_cjs_success_and_failure_contract() {
        use std::sync::Arc;
        use std::task::{Context, Poll, Wake, Waker};

        struct NoopWake;
        impl Wake for NoopWake {
            fn wake(self: Arc<Self>) {}
        }
        fn block_ready<F: Future>(future: F) -> F::Output {
            let waker = Waker::from(Arc::new(NoopWake));
            let mut context = Context::from_waker(&waker);
            let mut future = Box::pin(future);
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => value,
                Poll::Pending => panic!("test future unexpectedly pending"),
            }
        }

        let passed = block_ready(run_async_source_pipeline_check(
            "async-pass",
            "async details",
            || async { Ok(json!(["fixture", 7])) },
        ));
        assert_eq!(passed.status, "passed");
        assert_eq!(passed.details, ["fixture", "7"]);
        let failed = block_ready(run_async_source_pipeline_check(
            "async-fail",
            "async error",
            || async { Err("fixture failure".to_string()) },
        ));
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.details, ["fixture failure"]);
    }

    #[test]
    fn core_execution_preserves_cjs_check_order_and_failure_exit_status() {
        let check = |id: &str| SourcePipelineValidationCheck {
            id: id.to_string(),
            label: id.to_string(),
            status: "passed".to_string(),
            details: Vec::new(),
        };
        let artifact_checks = SourcePipelineArtifactAlignmentResult {
            promotion_report_check: check("promotion-report-matches-generated-registry"),
            promoted_owners_check: check("promoted-owners-parsed-and-fingerprinted"),
            canonical_parse_check: check("canonical-requirement-sources-parse-successfully"),
            fingerprint_alignment_check: check("fingerprint-alignment"),
            promoted_owner_coverage_repair: Value::Null,
            fingerprint_alignment_repair: Value::Null,
        };
        let eligible_checks = SourcePipelineEligiblePromotionChecks {
            eligible_discoveries_promoted_check: check("eligible-discoveries-promoted"),
            eligible_promotions_cleared_check: check(
                "eligible-promotions-cleared-from-review-and-gap-reports",
            ),
        };
        let relationship_checks = SourcePipelineRelationshipChecks {
            review_queue_alignment_check: check("review-queue-aligned-with-source-gaps"),
            promotion_materialization_check: check("promotions-materialized-in-canonical-registry"),
        };
        let parser_alignment = SourcePipelineParserAlignmentResult {
            check: check("registry-parser-alignment"),
            repair: Value::Null,
            canonical_parseable_source_key_count: 0,
            parser_output_left_only_count: 0,
            parser_output_right_only_count: 0,
        };
        let mut trailing_fixture_check = check("stale-year-replacement-trigger");
        trailing_fixture_check.status = "failed".to_string();
        trailing_fixture_check.details = vec!["fixture blocker".to_string()];
        let execution =
            build_source_pipeline_validation_execution(SourcePipelineValidationExecutionInput {
                generated_at: "2026-01-02T03:04:05.000Z".to_string(),
                metrics: SourcePipelineValidationMetrics::default(),
                discovery_partition_check: check("discovery-partition"),
                artifact_checks,
                eligible_checks,
                relationship_checks,
                parser_alignment,
                trailing_checks: vec![trailing_fixture_check],
            })
            .unwrap();
        assert_eq!(execution.exit_code, 1);
        assert_eq!(execution.report.outcome, "failed");
        assert_eq!(execution.report.passed_count, 10);
        assert_eq!(execution.report.failed_count, 1);
        let checks = &execution.report.checks;
        assert_eq!(
            checks
                .iter()
                .map(|check| check.id.as_str())
                .collect::<Vec<_>>(),
            [
                "discovery-partition",
                "promotion-report-matches-generated-registry",
                "eligible-discoveries-promoted",
                "review-queue-aligned-with-source-gaps",
                "promotions-materialized-in-canonical-registry",
                "registry-parser-alignment",
                "promoted-owners-parsed-and-fingerprinted",
                "canonical-requirement-sources-parse-successfully",
                "fingerprint-alignment",
                "eligible-promotions-cleared-from-review-and-gap-reports",
                "stale-year-replacement-trigger",
            ]
        );
    }

    #[test]
    fn json_report_loader_fails_closed_without_creating_missing_report_paths() {
        let root = Path::new("definitely-not-a-source-pipeline-fixture-root");
        let error = load_source_pipeline_json_reports(root).unwrap_err();
        let expected_path = root
            .join(".tmp")
            .join("reports")
            .join("transfer-planner-primary-source-discovery.json");
        assert_eq!(
            error,
            format!(
                "Missing primary-source discovery report at {}",
                expected_path.display()
            )
        );
        let input = json!({ "sourceRoot": root.to_string_lossy() });
        assert_eq!(
            build_source_pipeline_core_execution_from_json(&input)
                .err()
                .unwrap(),
            error
        );
        assert!(!root.join(".tmp").exists());
    }

    #[test]
    fn json_core_orchestrator_loads_cjs_report_paths_from_source_root() {
        let unique_stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let source_root = std::env::temp_dir().join(format!(
            "gator-guide-source-pipeline-input-{}-{unique_stamp}",
            std::process::id()
        ));
        assert!(!source_root.exists());
        let report_dir = source_root.join(".tmp").join("reports");
        fs::create_dir_all(&report_dir).unwrap();
        let report_fixtures = [
            (
                "transfer-planner-primary-source-discovery.json",
                json!({"owners": [], "weakExistingOwners": []}),
            ),
            (
                "transfer-planner-primary-source-review-queue.json",
                json!({"campuses": []}),
            ),
            (
                "transfer-planner-primary-source-promotions.json",
                json!({"totalPromotions": 0}),
            ),
            ("transfer-planner-source-gaps.json", json!({"entries": []})),
            (
                "transfer-planner-requirement-source-parse-report.json",
                json!({"owners": [], "totalOwners": 0}),
            ),
            (
                "transfer-planner-source-fingerprints.json",
                json!({"totalRequirementSourceFingerprints": 0}),
            ),
        ];
        for (name, value) in report_fixtures {
            fs::write(report_dir.join(name), serde_json::to_vec(&value).unwrap()).unwrap();
        }
        let input = json!({
            "generatedAt": "2026-01-02T03:04:05.000Z",
            "sourceRoot": source_root.to_string_lossy(),
            "validationTargetPlanId": null,
            "canonicalPrimaryManifest": [],
            "parseablePrimaryManifest": [],
            "parseableRequirementSourceEntries": [],
            "promotions": [],
            "generatedParsedBlocks": [],
            "generatedRequirementFingerprints": [],
            "activePathways": [],
            "generatedPlans": [],
            "manualOverrides": [],
            "runtimeAliasCoveredOwnerIds": [],
            "runtimePathwayRecords": [],
            "structurallySuspiciousOwnerKeys": []
        });
        let execution = build_source_pipeline_core_execution_from_json(&input).unwrap();
        assert_eq!(execution.exit_code, 0);
        assert_eq!(execution.report.outcome, "passed");
        assert_eq!(execution.report.checks.len(), 12);
        assert_eq!(
            execution.report.checks[10].id,
            "stale-year-replacement-trigger"
        );
        assert_eq!(
            execution.report.checks[11].id,
            "weak-existing-primary-replacement-trigger"
        );
        fs::remove_dir_all(source_root).unwrap();
    }

    #[test]
    #[ignore = "requires Node and the canonical TypeScript runtime; run explicitly for bridge integration"]
    fn canonical_context_bridge_reads_existing_typescript_registries_offline() {
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("source");
        let context = load_source_pipeline_canonical_context(
            &source_root,
            &json!({"owners": [], "weakExistingOwners": []}),
            &json!({"campuses": []}),
        )
        .unwrap();
        assert!(
            context["canonicalPrimaryManifest"]
                .as_array()
                .unwrap()
                .len()
                > 0
        );
        assert!(
            context["parseableRequirementSourceEntries"]
                .as_array()
                .unwrap()
                .len()
                > 0
        );
        assert!(context["activePathways"].as_array().unwrap().len() > 0);
        assert!(context["generatedPlans"].as_array().unwrap().len() > 0);
        assert!(context["generatedParsedBlocks"].as_array().unwrap().len() > 0);
    }

    #[test]
    fn parser_backed_review_owners_are_derived_from_generated_block_labels() {
        let pathway = json!({
            "id": "plan:pathway:computer-science",
            "planId": "plan",
            "pathwayId": "computer-science"
        });
        let block = json!({
            "ok": true,
            "planId": "plan",
            "pathwayLabels": ["Computer Science"]
        });
        let keys = build_parser_backed_review_owner_keys(&[block], &[pathway]);
        assert!(keys.contains("plan"));
        assert!(keys.contains("plan:pathway:computer-science"));
    }

    #[test]
    fn json_core_orchestrator_requires_complete_input_and_builds_report() {
        let incomplete = json!({"generatedAt": "2026-01-02T03:04:05.000Z"});
        assert_eq!(
            build_source_pipeline_core_execution_from_json(&incomplete)
                .err()
                .unwrap(),
            "source-pipeline input is missing discoveryReport"
        );
        let input = json!({
            "generatedAt": "2026-01-02T03:04:05.000Z",
            "validationTargetPlanId": null,
            "discoveryReport": {"owners": [], "weakExistingOwners": []},
            "reviewQueue": {"campuses": []},
            "promotionReport": {"totalPromotions": 0},
            "sourceGapReport": {"entries": []},
            "requirementParseReport": {"owners": [], "totalOwners": 0},
            "sourceFingerprintReport": {"totalRequirementSourceFingerprints": 0},
            "canonicalPrimaryManifest": [],
            "parseablePrimaryManifest": [],
            "parseableRequirementSourceEntries": [],
            "promotions": [],
            "generatedParsedBlocks": [],
            "generatedRequirementFingerprints": [],
            "activePathways": [],
            "generatedPlans": [],
            "manualOverrides": [],
            "runtimeAliasCoveredOwnerIds": [],
            "runtimePathwayRecords": [],
            "structurallySuspiciousOwnerKeys": []
        });
        let execution = build_source_pipeline_core_execution_from_json(&input).unwrap();
        assert_eq!(execution.exit_code, 0);
        assert_eq!(execution.report.outcome, "passed");
        assert_eq!(execution.report.checks.len(), 12);
        assert!(
            execution.report.checks[10..]
                .iter()
                .all(|check| check.status == "passed")
        );
    }

    #[test]
    fn core_orchestrator_builds_all_sync_checks_from_loaded_json_inputs() {
        let discovery = json!({"owners": [], "weakExistingOwners": []});
        let review = json!({"campuses": []});
        let promotion_report = json!({"totalPromotions": 0});
        let source_gaps = json!({"entries": []});
        let parse_report = json!({"owners": [], "totalOwners": 0});
        let fingerprint_report = json!({"totalRequirementSourceFingerprints": 0});
        let empty_values: [Value; 0] = [];
        let parser_backed = HashSet::new();
        let active_pathways: [Value; 0] = [];
        let generated_plans: [Value; 0] = [];
        let manual_overrides: [Value; 0] = [];
        let alias_covered = HashSet::new();
        let runtime_pathways: [Value; 0] = [];
        let suspicious_keys = HashSet::new();
        let promotion_context = SourcePipelinePromotionContext {
            generated_plans: &generated_plans,
            manual_overrides: &manual_overrides,
            runtime_alias_covered_owner_ids: &alias_covered,
            runtime_pathway_records: &runtime_pathways,
            structurally_suspicious_owner_keys: &suspicious_keys,
        };
        let execution = build_source_pipeline_core_execution(
            SourcePipelineValidationInput {
                validation_target_plan_id: None,
                discovery_report: &discovery,
                review_queue: &review,
                promotion_report: &promotion_report,
                source_gap_report: &source_gaps,
                requirement_parse_report: &parse_report,
                source_fingerprint_report: &fingerprint_report,
                canonical_primary_manifest: &empty_values,
                parseable_primary_manifest: &empty_values,
                parseable_requirement_source_entries: &empty_values,
                promotions: &empty_values,
                generated_parsed_blocks: &empty_values,
                generated_requirement_fingerprints: &empty_values,
                parser_backed_review_owner_keys: &parser_backed,
                active_pathways: &active_pathways,
                promotion_context: &promotion_context,
            },
            "2026-01-02T03:04:05.000Z",
            Vec::new(),
        )
        .unwrap();
        assert_eq!(execution.exit_code, 0);
        assert_eq!(execution.report.outcome, "passed");
        assert_eq!(execution.report.checks.len(), 10);
        assert_eq!(
            execution
                .report
                .checks
                .iter()
                .map(|check| check.id.as_str())
                .collect::<Vec<_>>(),
            [
                "discovery-partition",
                "promotion-report-matches-generated-registry",
                "eligible-discoveries-promoted",
                "review-queue-aligned-with-source-gaps",
                "promotions-materialized-in-canonical-registry",
                "registry-parser-alignment",
                "promoted-owners-parsed-and-fingerprinted",
                "canonical-requirement-sources-parse-successfully",
                "fingerprint-alignment",
                "eligible-promotions-cleared-from-review-and-gap-reports",
            ]
        );
    }

    #[test]
    fn report_artifact_renderer_preserves_json_order_and_writer_newlines() {
        let report = build_source_pipeline_validation_report(
            "2026-01-02T03:04:05.000Z",
            SourcePipelineValidationMetrics::default(),
            json!({
                "registryParserAlignment": {"needed": false},
                "promotedOwnerCoverage": {"needed": false},
                "fingerprintAlignment": {"needed": false}
            }),
            vec![SourcePipelineValidationCheck {
                id: "fixture".to_string(),
                label: "Candidate | source".to_string(),
                status: "passed".to_string(),
                details: vec!["detail\\|value".to_string()],
            }],
        );
        let artifacts = render_source_pipeline_report_artifacts(&report).unwrap();
        assert!(artifacts.json.starts_with(
            "{\n  \"generatedAt\": \"2026-01-02T03:04:05.000Z\",\n  \"outcome\": \"passed\""
        ));
        assert!(artifacts.json.ends_with('\n'));
        assert_eq!(
            serde_json::from_str::<Value>(&artifacts.json).unwrap(),
            serde_json::to_value(&report).unwrap()
        );
        assert!(artifacts.markdown.ends_with('\n'));
        assert!(artifacts.markdown.contains("Candidate \\| source"));
    }

    #[test]
    fn explicit_report_writer_uses_cjs_paths_and_serialized_artifacts() {
        let unique_stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let source_root = std::env::temp_dir().join(format!(
            "gator-guide-source-pipeline-writer-{}-{unique_stamp}",
            std::process::id()
        ));
        assert!(!source_root.exists());
        let report = build_source_pipeline_validation_report(
            "2026-01-02T03:04:05.000Z",
            SourcePipelineValidationMetrics::default(),
            json!({"registryParserAlignment": {"needed": false}}),
            Vec::new(),
        );
        let (json_path, markdown_path) =
            write_source_pipeline_report_artifacts(&source_root, &report).unwrap();
        assert_eq!(
            json_path,
            source_root.join(".tmp/reports/transfer-planner-source-pipeline-validation.json")
        );
        assert_eq!(
            markdown_path,
            source_root.join(".tmp/reports/transfer-planner-source-pipeline-validation.md")
        );
        let json_text = fs::read_to_string(&json_path).unwrap();
        assert!(json_text.ends_with('\n'));
        assert_eq!(
            serde_json::from_str::<Value>(&json_text).unwrap(),
            serde_json::to_value(&report).unwrap()
        );
        assert!(fs::read_to_string(&markdown_path).unwrap().ends_with('\n'));
        fs::remove_dir_all(source_root).unwrap();
    }

    #[test]
    fn check_runner_and_report_builder_match_cjs_status_and_count_contract() {
        let passed = run_source_pipeline_check("ok", "Passed", || Ok(json!(["source", 7, null])));
        let failed = run_source_pipeline_check("bad", "Failed", || Err("fixture mismatch".into()));
        assert_eq!(passed.details, vec!["source", "7", "null"]);
        assert_eq!(failed.status, "failed");
        assert_eq!(failed.details, vec!["fixture mismatch"]);

        let report = build_source_pipeline_validation_report(
            "2026-01-02T03:04:05.000Z",
            SourcePipelineValidationMetrics::default(),
            json!({"registryParserAlignment": {"needed": false}}),
            vec![passed, failed],
        );
        assert_eq!(report.outcome, "failed");
        assert_eq!(report.passed_count, 1);
        assert_eq!(report.failed_count, 1);
        let serialized = serde_json::to_string(&report).unwrap();
        let keys = [
            "generatedAt",
            "outcome",
            "passedCount",
            "failedCount",
            "metrics",
            "autoRepair",
            "checks",
        ];
        let positions = keys
            .iter()
            .map(|key| serialized.find(&format!("\"{key}\"")).unwrap())
            .collect::<Vec<_>>();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn set_differences_match_javascript_set_and_default_sort_semantics() {
        let left = vec![
            "x".to_string(),
            "x".to_string(),
            "\u{E000}".to_string(),
            "😀".to_string(),
        ];
        let right = vec!["x".to_string(), "other".to_string()];
        assert_eq!(
            compare_string_sets(&left, &right),
            (
                vec!["😀".to_string(), "\u{E000}".to_string()],
                vec!["other".to_string()]
            )
        );
    }

    #[test]
    fn markdown_report_matches_cjs_heading_metrics_escaping_and_newline_contract() {
        let report = json!({
            "generatedAt": "2026-01-02T03:04:05.006Z",
            "outcome": "passed",
            "passedCount": 1,
            "failedCount": 0,
            "metrics": {
                "validationTargetPlanId": null,
                "eligibleAutoPromotionOwnerCount": 2,
                "weakExistingOwnerCount": 3,
                "highConfidenceReplacementOwnerCount": 4,
                "reviewQueueOwnerCount": 5,
                "sourceGapOwnerCount": 6,
                "promotedOwnerCount": 7,
                "parseablePrimaryOwnerCount": 8,
                "parsedOwnerCount": 9,
                "requirementFingerprintOwnerCount": 10
            },
            "checks": [{
                "label": "Check | A\\\\B",
                "status": "passed",
                "details": ["detail | \\\\path"]
            }]
        });
        assert_eq!(
            render_markdown_report(&report),
            "# Transfer Planner Source Pipeline Validation\n\nGenerated: 2026-01-02T03:04:05.006Z\n\n- Outcome: passed\n- Passed checks: 1\n- Failed checks: 0\n- Validation target: all owners\n- Eligible auto-promotions from discovery: 2\n- Weak existing primaries re-evaluated: 3\n- High-confidence replacements: 4\n- Review-queue owners: 5\n- Source-gap owners: 6\n- Promoted owners in canonical registry: 7\n- Parseable primary owners: 8\n- Parsed owners: 9\n- Requirement fingerprints: 10\n\n| Check | Status | Details |\n| --- | --- | --- |\n| Check \\| A\\\\\\\\B | passed | detail \\| \\\\\\\\path |\n\n"
        );
    }

    #[test]
    fn target_plan_flags_match_cjs_aliases_csv_and_single_target_validation() {
        let args = [
            "--plan-id".to_string(),
            "beta".to_string(),
            "--target-plan-ids=alpha, beta, ,alpha".to_string(),
        ];
        assert_eq!(
            parse_target_plan_ids(&args),
            Err(
                "Source pipeline validation supports one target plan at a time; received alpha, beta."
                    .to_string()
            )
        );

        let single_target = [
            "--target-plan-ids".to_string(),
            "\u{FEFF}alpha , alpha\u{FEFF}".to_string(),
        ];
        assert_eq!(
            parse_target_plan_ids(&single_target),
            Ok(vec!["alpha".to_string()])
        );

        let empty_target = ["--target-plan-id=".to_string()];
        assert_eq!(parse_target_plan_ids(&empty_target), Ok(Vec::new()));
    }

    #[test]
    fn specialized_source_matching_and_candidate_projection_follow_cjs_rules() {
        let generated_plans = [
            json!({
                "id": "specialized",
                "title": "Biology: Microbiology (BS)",
                "officialLinks": [
                    {"url": "https://example.edu/degree/?source=guide"},
                    {"url": "https://EXAMPLE.edu/degree#requirements"}
                ]
            }),
            json!({
                "id": "non-unique",
                "title": "Biology: Microbiology (BS)",
                "officialLinks": [
                    {"url": "https://example.edu/degree"},
                    {"url": "https://example.edu/catalog"}
                ]
            }),
        ];
        assert_eq!(
            get_single_specialized_plan_official_url(Some("specialized"), &generated_plans),
            Some("https://example.edu/degree".to_string())
        );
        assert!(suggested_primary_matches_specialized_plan_source(
            &json!({"planId": "specialized", "suggestedPrimary": {"url": "https://example.edu/degree/?x=1#section"}}),
            &generated_plans
        ));
        assert!(!suggested_primary_matches_specialized_plan_source(
            &json!({"planId": "specialized", "suggestedPrimary": {"url": "https://example.edu/catalog"}}),
            &generated_plans
        ));
        assert!(suggested_primary_matches_specialized_plan_source(
            &json!({"planId": "non-unique", "suggestedPrimary": {"url": "https://example.edu/other"}}),
            &generated_plans
        ));
        assert_eq!(
            normalize_promotion_url_for_comparison(Some(&json!(" not a url?x=1/ "))),
            "not a url"
        );

        let candidate = build_promotion_entry_candidate_from_owner(&json!({
            "ownerType": "major",
            "planId": "plan",
            "title": "Sample Major",
            "campusId": "uw-seattle",
            "suggestedPrimary": {
                "label": "",
                "anchorText": "Degree requirements",
                "url": "https://example.edu/requirements"
            }
        }));
        assert_eq!(candidate["ownerId"], "plan");
        assert_eq!(candidate["label"], "Degree requirements");
        assert_eq!(candidate["ownerTitle"], "Sample Major");
        assert_eq!(candidate["pathwayId"], Value::Null);
    }

    #[test]
    fn promotion_safety_predicates_cover_support_routes_and_known_cross_major_leaks() {
        assert!(is_clearly_support_only_promotion_entry(&json!({
            "label": "Undergraduate advising resources"
        })));
        assert!(!is_clearly_support_only_promotion_entry(&json!({
            "label": "Undergraduate degree requirements"
        })));
        assert!(has_document_url_with_appended_path(&json!({
            "url": "https://example.edu/degree.PDF/extra"
        })));
        assert!(!has_document_url_with_appended_path(&json!({
            "url": "https://example.edu/degree.pdf?download=1"
        })));
        assert!(is_minor_credential_promotion_for_major_owner(&json!({
            "ownerType": "major",
            "url": "https://catalog.example/program#credential-42",
            "label": "Minor in History"
        })));
        assert!(!is_minor_credential_promotion_for_major_owner(&json!({
            "ownerType": "major",
            "pathwayId": "history-option",
            "url": "https://catalog.example/program#credential-42",
            "label": "Minor in History"
        })));
        assert!(is_skip_navigation_promotion_entry(&json!({
            "label": "Skip to main content"
        })));

        let ba_bs_plans = [json!({
            "id": "plan",
            "pathways": [{"label": "BS Option"}]
        })];
        assert!(is_ba_bs_route_promotion_for_broad_major_owner(
            &json!({"ownerType": "major", "planId": "plan", "label": "BS Checklist"}),
            &ba_bs_plans
        ));
        assert!(is_unscoped_acs_route_promotion_for_broad_major_owner(
            &json!({
                "ownerType": "major",
                "ownerId": "chemistry",
                "planId": "chemistry",
                "ownerTitle": "Chemistry",
                "label": "ACS certified route"
            })
        ));
        assert!(!is_unscoped_acs_route_promotion_for_broad_major_owner(
            &json!({
                "ownerType": "major",
                "ownerId": "acs-chemistry",
                "planId": "acs-chemistry",
                "ownerTitle": "ACS Chemistry",
                "label": "ACS certified route"
            })
        ));
        assert!(is_tacoma_global_studies_leaked_promotion_entry(&json!({
            "campusId": "uw-tacoma",
            "planId": "uw-tacoma-history",
            "pathwayId": "global-studies-concentration"
        })));
        assert!(is_seattle_jsis_asia_concentration_leaked_promotion_entry(
            &json!({
                "campusId": "uw-seattle",
                "planId": "uw-seattle-international-studies",
                "pathwayId": "japan-concentration",
                "label": "Asian Studies Japan concentration"
            })
        ));
    }

    #[test]
    fn catalog_promotion_safety_matches_major_titles_and_catalog_pathways() {
        let plans = [
            json!({"id": "chemistry", "title": "Chemistry: Biochemistry"}),
            json!({"id": "uw-seattle-european-studies", "title": "European Studies"}),
            json!({"id": "broad", "title": "Chemistry"}),
        ];
        assert_eq!(
            normalize_catalog_credential_major_name(Some(&json!(
                "Bachelor of Chemistry (Option) & Biology"
            ))),
            "chemistry and biology"
        );
        let chemistry_credential = json!({
            "planId": "chemistry",
            "url": "https://catalog.example/#credential-91",
            "label": "Major in Biology: Admission requirements"
        });
        assert!(is_catalog_credential_promotion_entry(&chemistry_credential));
        assert!(is_catalog_credential_promotion_for_different_major(
            &chemistry_credential,
            &plans
        ));
        assert!(!is_catalog_credential_promotion_for_different_major(
            &json!({
                "planId": "chemistry",
                "url": "https://catalog.example/#credential-91",
                "label": "Major in Chemistry: Degree requirements"
            }),
            &plans
        ));
        let program = json!({
            "planId": "chemistry",
            "url": "https://catalog.example/#program-17",
            "label": "Program of Study: Major: Biology"
        });
        assert!(is_catalog_program_promotion_entry(&program));
        assert!(is_catalog_program_promotion_for_different_major(
            &program, &plans
        ));
        assert!(is_catalog_program_page_promotion_for_different_major(
            &json!({
                "planId": "chemistry",
                "url": "https://catalog.example/students/gencat/program/biology",
                "label": "Applied Biology"
            }),
            &plans
        ));
        assert!(!is_catalog_program_page_promotion_for_different_major(
            &json!({
                "planId": "chemistry",
                "url": "https://catalog.example/students/gencat/program/chemistry",
                "label": "Applied Chemistry"
            }),
            &plans
        ));
        assert!(
            is_pathway_scoped_catalog_credential_promotion_for_broad_major_owner(
                &json!({
                    "ownerType": "major",
                    "planId": "broad",
                    "ownerTitle": "Chemistry",
                    "url": "https://catalog.example/#credential-2",
                    "label": "Major in Chemistry: Biochemistry option"
                }),
                &plans
            )
        );
        assert!(
            !is_pathway_scoped_catalog_credential_promotion_for_broad_major_owner(
                &json!({
                    "ownerType": "major",
                    "planId": "broad",
                    "pathwayId": "biochemistry",
                    "url": "https://catalog.example/#credential-2",
                    "label": "Major in Chemistry: Biochemistry option"
                }),
                &plans
            )
        );
        assert!(!is_catalog_program_promotion_for_different_major(
            &json!({
                "planId": "uw-seattle-european-studies",
                "url": "https://catalog.example/#program-3",
                "label": "Program of Study: Major: International Studies: Europe"
            }),
            &plans
        ));
    }

    #[test]
    fn combined_promotion_safety_gate_preserves_each_veto_source() {
        let plans = [json!({
            "id": "chemistry",
            "title": "Chemistry",
            "pathways": []
        })];
        let unsafe_catalog_owner = json!({
            "ownerType": "major",
            "planId": "chemistry",
            "title": "Chemistry",
            "suggestedPrimary": {
                "url": "https://catalog.example/#credential-5",
                "label": "Major in Biology: Requirements"
            }
        });
        assert!(!suggested_primary_is_safe_for_automatic_promotion(
            &unsafe_catalog_owner,
            &plans,
            &[],
            false
        ));
        let safe_owner = json!({
            "ownerType": "major",
            "planId": "chemistry",
            "title": "Chemistry",
            "suggestedPrimary": {
                "url": "https://chemistry.example.edu/degree-requirements",
                "label": "Degree requirements"
            }
        });
        assert!(suggested_primary_is_safe_for_automatic_promotion(
            &safe_owner,
            &plans,
            &[],
            false
        ));
        let candidate = build_promotion_entry_candidate_from_owner(&json!({
            "ownerType": "pathway",
            "planId": "chemistry",
            "pathwayId": "option",
            "title": "Chemistry Option",
            "suggestedPrimary": {
                "url": "https://chemistry.example.edu/option",
                "label": "Option requirements"
            }
        }));
        assert!(is_unsafe_automatic_promotion_entry(
            &candidate,
            &plans,
            &[],
            false
        ));
        assert!(is_suspicious_structural_pathway_promotion_entry(&json!({
            "ownerType": "pathway",
            "pathwayId": "page-3",
            "label": "Degree requirements"
        })));
        assert!(!is_suspicious_structural_pathway_promotion_entry(&json!({
            "ownerType": "pathway",
            "pathwayId": "data-science-option",
            "label": "Data Science Option"
        })));
        assert!(is_suspicious_structural_pathway_promotion_entry(&json!({
            "ownerType": "pathway",
            "pathwayId": "double-major-page",
            "label": "Double major requirements"
        })));
        for pathway_id in [
            "2-of-the-4-credits-required-for-the-concentration",
            "with-the-option-to-graduate-early",
            "choose-from-the-following-options",
            "minimum-90-credits-required",
        ] {
            assert!(
                is_suspicious_structural_pathway_promotion_entry(&json!({
                    "ownerType": "pathway",
                    "pathwayId": pathway_id,
                    "label": "Ordinary label"
                })),
                "expected structural ID to be rejected: {pathway_id}"
            );
        }
        assert!(!is_suspicious_structural_pathway_promotion_entry(&json!({
            "ownerType": "pathway",
            "pathwayId": "bs-option-family:general-biology",
            "label": "Ordinary label"
        })));
        assert!(is_unsafe_automatic_promotion_entry(
            &build_promotion_entry_candidate_from_owner(&safe_owner),
            &plans,
            &[json!({"planId": "chemistry", "mode": "replace"})],
            false
        ));
    }

    #[test]
    fn missing_primary_block_reason_uses_cjs_gate_order_and_exact_messages() {
        let plans = [
            json!({"id": "chemistry", "title": "Chemistry"}),
            json!({
                "id": "specialized",
                "title": "Chemistry: Biochemistry",
                "officialLinks": [{"url": "https://chemistry.example.edu/official"}]
            }),
        ];
        let weak_suggestion = json!({
            "planId": "chemistry",
            "suggestedPrimary": {"confidence": "medium"}
        });
        assert_eq!(
            get_missing_primary_auto_promotion_block_reason(&weak_suggestion, &plans, &[], false),
            Some("suggested primary is not a high-confidence schedulable primary")
        );
        let mismatched_specialized_source = json!({
            "planId": "specialized",
            "suggestedPrimary": {
                "confidence": "high",
                "url": "https://chemistry.example.edu/other",
                "label": "Degree requirements"
            }
        });
        assert_eq!(
            get_missing_primary_auto_promotion_block_reason(
                &mismatched_specialized_source,
                &plans,
                &[],
                false
            ),
            Some("suggested primary does not match the specialized plan source")
        );
        let unsafe_catalog = json!({
            "ownerType": "major",
            "planId": "chemistry",
            "title": "Chemistry",
            "suggestedPrimary": {
                "confidence": "high",
                "url": "https://catalog.example/#credential-5",
                "label": "Major in Biology: Requirements"
            }
        });
        assert_eq!(
            get_missing_primary_auto_promotion_block_reason(&unsafe_catalog, &plans, &[], false),
            Some("suggested primary is catalog-scoped to a different major or child option")
        );
        let eligible = json!({
            "planId": "chemistry",
            "suggestedPrimary": {
                "confidence": "high",
                "url": "https://chemistry.example.edu/degree-requirements",
                "label": "Degree requirements"
            }
        });
        assert_eq!(
            get_missing_primary_auto_promotion_block_reason(&eligible, &plans, &[], false),
            None
        );
    }

    #[test]
    fn automatic_promotion_selection_respects_review_alias_primary_and_weak_replacement_gates() {
        let discovery = json!({
            "owners": [
                {
                    "ownerType": "major", "ownerKey": "eligible", "planId": "eligible",
                    "title": "Eligible", "suggestedPrimary": {
                        "confidence": "high", "url": "https://eligible.example.edu/requirements",
                        "label": "Degree requirements"
                    }
                },
                {
                    "ownerType": "major", "ownerKey": "blocked", "planId": "blocked",
                    "title": "Blocked", "suggestedPrimary": {
                        "confidence": "medium", "url": "https://blocked.example.edu/requirements",
                        "label": "Degree requirements"
                    }
                },
                {
                    "ownerType": "major", "ownerKey": "reviewed", "planId": "reviewed",
                    "title": "Reviewed", "suggestedPrimary": {
                        "confidence": "high", "url": "https://reviewed.example.edu/requirements",
                        "label": "Degree requirements"
                    }
                },
                {
                    "ownerType": "major", "ownerKey": "covered", "planId": "covered",
                    "title": "Covered", "suggestedPrimary": {
                        "confidence": "high", "url": "https://covered.example.edu/requirements",
                        "label": "Degree requirements"
                    }
                },
                {
                    "ownerType": "major", "ownerKey": "existing", "planId": "existing",
                    "title": "Existing", "existingPrimaryUrl": "https://existing.example.edu",
                    "suggestedPrimary": {
                        "confidence": "high", "url": "https://existing.example.edu/requirements",
                        "label": "Degree requirements"
                    }
                }
            ],
            "weakExistingOwners": [
                {
                    "ownerType": "major", "ownerKey": "weak-reviewed", "planId": "weak-reviewed",
                    "title": "Weak reviewed", "suggestedAction": "replace-existing-primary",
                    "currentPrimary": {"sourceRoleStatus": "support"},
                    "suggestedPrimary": {
                        "confidence": "high", "url": "https://weak-reviewed.example.edu/requirements",
                        "label": "Degree requirements"
                    }
                },
                {
                    "ownerType": "major", "ownerKey": "weak-clear", "planId": "weak-clear",
                    "title": "Weak clear", "suggestedAction": "replace-existing-primary",
                    "suggestedPrimary": {
                        "confidence": "high", "url": "https://weak-clear.example.edu/requirements",
                        "label": "Degree requirements"
                    }
                }
            ]
        });
        let review_queue = json!({"campuses": [{"entries": [
            {"ownerType": "major", "ownerKey": "reviewed", "planId": "reviewed"},
            {"ownerType": "major", "ownerKey": "weak-reviewed", "planId": "weak-reviewed"}
        ]}]});
        let generated_plans = [
            json!({"id": "eligible", "title": "Eligible"}),
            json!({"id": "blocked", "title": "Blocked"}),
            json!({"id": "reviewed", "title": "Reviewed"}),
            json!({"id": "covered", "title": "Covered"}),
            json!({"id": "existing", "title": "Existing"}),
            json!({"id": "weak-reviewed", "title": "Weak reviewed"}),
            json!({"id": "weak-clear", "title": "Weak clear"}),
        ];
        let manual_overrides = [];
        let runtime_alias_covered_owner_ids = HashSet::from(["covered".to_string()]);
        let runtime_pathway_records = [];
        let structurally_suspicious_owner_keys = HashSet::new();
        let context = SourcePipelinePromotionContext {
            generated_plans: &generated_plans,
            manual_overrides: &manual_overrides,
            runtime_alias_covered_owner_ids: &runtime_alias_covered_owner_ids,
            runtime_pathway_records: &runtime_pathway_records,
            structurally_suspicious_owner_keys: &structurally_suspicious_owner_keys,
        };
        let eligible = get_eligible_missing_primary_auto_promotion_owners(
            &discovery,
            &review_queue,
            None,
            &context,
        );
        assert_eq!(eligible.len(), 1);
        assert_eq!(eligible[0]["ownerId"], "eligible");
        let blocked = get_blocked_missing_primary_auto_promotion_owners(
            &discovery,
            &review_queue,
            None,
            &context,
        );
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0]["ownerId"], "blocked");
        assert_eq!(
            blocked[0]["blockReason"],
            "suggested primary is not a high-confidence schedulable primary"
        );
        let weak = get_eligible_weak_existing_replacement_owners(
            &discovery,
            &review_queue,
            None,
            &context,
        );
        assert_eq!(
            weak.iter()
                .map(|owner| owner["ownerId"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["weak-reviewed", "weak-clear"]
        );
        let all_eligible =
            get_eligible_auto_promotion_owners(&discovery, &review_queue, None, &context);
        assert_eq!(
            all_eligible
                .iter()
                .map(|owner| owner["ownerId"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["eligible", "weak-clear", "weak-reviewed"]
        );
    }

    #[test]
    fn derived_parent_pathway_alias_coverage_matches_generated_runtime_records() {
        let owner = json!({
            "ownerType": "major",
            "planId": "computer-science-ai",
            "campusId": "uw-seattle",
            "title": "Computer Science: Artificial Intelligence (BA)"
        });
        let parsed = parse_derived_option_alias_title(owner.get("title"));
        assert_eq!(
            parsed,
            Some(DerivedOptionAliasTitle {
                parent_title: "Computer Science (BA)".to_string(),
                option_title: "Artificial Intelligence".to_string()
            })
        );
        let generated_plans = [json!({
            "id": "computer-science-parent",
            "campusId": "uw-seattle",
            "title": "Computer Science (BA)"
        })];
        let runtime_pathways = [json!({
            "planId": "computer-science-parent",
            "pathways": [{"id": "artificial-intelligence-option", "label": "AI pathway"}]
        })];
        assert!(has_derived_parent_pathway_runtime_coverage(
            &owner,
            &generated_plans,
            &runtime_pathways
        ));
        assert!(!has_derived_parent_pathway_runtime_coverage(
            &json!({
                "ownerType": "pathway",
                "campusId": "uw-seattle",
                "title": "Computer Science: Artificial Intelligence (BA)"
            }),
            &generated_plans,
            &runtime_pathways
        ));
        assert!(!has_derived_parent_pathway_runtime_coverage(
            &json!({
                "ownerType": "major",
                "campusId": "uw-bothell",
                "title": "Computer Science: Artificial Intelligence (BA)"
            }),
            &generated_plans,
            &runtime_pathways
        ));
        assert!(!has_derived_parent_pathway_runtime_coverage(
            &owner,
            &generated_plans,
            &[]
        ));
        assert!(has_student_runtime_alias_coverage(
            &owner,
            &HashSet::new(),
            &generated_plans,
            &runtime_pathways
        ));
        let directly_covered_owner = json!({"planId": "direct-alias-plan"});
        let direct_owner_id = super::super::source_fingerprints::build_transfer_planner_owner_id(
            Some("direct-alias-plan"),
            None,
        );
        assert!(has_student_runtime_alias_coverage(
            &directly_covered_owner,
            &HashSet::from([direct_owner_id]),
            &[],
            &[]
        ));
        assert!(!has_student_runtime_alias_coverage(
            &json!({}),
            &HashSet::new(),
            &generated_plans,
            &runtime_pathways
        ));
    }

    #[test]
    fn manual_override_promotion_skips_respect_owner_scope_mode_preference_and_removals() {
        let overrides = [
            json!({
                "planId": "plan",
                "preferredPrimaryUrl": "https://example.edu/preferred/",
                "removedUrls": ["https://example.edu/removed/"]
            }),
            json!({"planId": "plan", "pathwayId": "route", "mode": "replace"}),
        ];
        assert!(should_skip_manual_override_promotion(
            Some(&json!("plan")),
            None,
            Some(&json!(" https://example.edu/other/\u{FEFF}")),
            &overrides
        ));
        assert!(!should_skip_manual_override_promotion(
            Some(&json!("plan")),
            None,
            Some(&json!("https://example.edu/preferred/")),
            &overrides
        ));
        assert!(should_skip_manual_override_promotion(
            Some(&json!("plan")),
            None,
            Some(&json!("https://example.edu/removed/")),
            &overrides
        ));
        assert!(should_skip_manual_override_promotion(
            Some(&json!("plan")),
            Some(&json!("route")),
            Some(&json!("https://example.edu/anything")),
            &overrides
        ));
        assert!(!should_skip_manual_override_promotion(
            Some(&json!("plan")),
            Some(&json!("other-route")),
            Some(&json!("https://example.edu/anything")),
            &overrides
        ));
        assert!(!should_skip_manual_override_promotion(
            Some(&json!("plan")),
            None,
            Some(&json!("  \u{FEFF}  ")),
            &overrides
        ));
    }

    #[test]
    fn parseable_manifest_selection_applies_plan_parser_and_owner_order_rules() {
        let manifest = [
            json!({"ownerId": "zeta", "ownerType": "major", "campusId": "uw-seattle", "planId": "zeta", "ownerTitle": "Zulu Major", "isPrimaryDegreeRequirementsLink": true, "parserType": "html-degree-page", "url": "https://example.edu/zulu"}),
            json!({"ownerId": "alpha", "ownerType": "major", "campusId": "uw-bothell", "planId": "alpha", "ownerTitle": "Alpha Major", "isPrimaryDegreeRequirementsLink": true, "parserType": "pdf-degree-sheet", "url": "https://example.edu/alpha"}),
            json!({"ownerId": "unsupported", "ownerType": "major", "campusId": "uw-seattle", "planId": "unsupported", "ownerTitle": "Unsupported Major", "isPrimaryDegreeRequirementsLink": true, "parserType": "custom-parser", "url": "https://example.edu/unsupported"}),
            json!({"ownerId": "grc", "ownerType": "major", "campusId": "grc", "planId": "grc", "ownerTitle": "GRC Major", "isPrimaryDegreeRequirementsLink": true, "parserType": "html-degree-page", "url": "https://example.edu/grc"}),
        ];
        let entries = build_parseable_requirement_source_manifest_entries(
            &manifest,
            &HashMap::new(),
            &CourseCodeParser::default(),
        );
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry["ownerId"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["alpha", "zeta"]
        );
    }

    #[test]
    fn manifest_owner_sets_match_primary_and_parseable_filters() {
        let manifest = [
            json!({"ownerId": "major", "ownerType": "major", "campusId": "uw-seattle", "isPrimaryDegreeRequirementsLink": true, "parserType": "html-degree-page"}),
            json!({"ownerId": "major", "ownerType": "major", "campusId": "uw-seattle", "isPrimaryDegreeRequirementsLink": true, "parserType": "html-degree-page"}),
            json!({"ownerId": "support", "ownerType": "major", "campusId": "uw-seattle", "isPrimaryDegreeRequirementsLink": true, "parserType": "unsupported"}),
            json!({"ownerId": "grc", "ownerType": "major", "campusId": "grc", "isPrimaryDegreeRequirementsLink": true, "parserType": "html-degree-page"}),
            json!({"ownerId": "pathway", "ownerType": "pathway", "campusId": "uw-bothell", "isPrimaryDegreeRequirementsLink": true, "parserType": "generic-pdf"}),
            json!({"ownerId": "not-primary", "ownerType": "pathway", "campusId": "uw-bothell", "isPrimaryDegreeRequirementsLink": false, "parserType": "generic-pdf"}),
        ];
        assert_eq!(
            build_primary_manifest_owner_ids(&manifest),
            HashSet::from([
                "major".to_string(),
                "support".to_string(),
                "pathway".to_string()
            ])
        );
        assert_eq!(
            build_parseable_primary_manifest_owner_ids(&manifest),
            HashSet::from(["major".to_string(), "pathway".to_string()])
        );
    }

    #[test]
    fn review_queue_and_discovery_helpers_preserve_owner_scope_rules() {
        let review_queue = json!({
            "campuses": [
                {"entries": [
                    {"ownerKey": "", "ownerId": "uw-seattle-biology", "planId": "uw-seattle-biology"},
                    {"ownerId": "uw-seattle-biology:pathway:general", "planId": "uw-seattle-biology", "pathwayId": "general"}
                ]},
                {"entries": [{"ownerId": "uw-bothell-biology", "planId": "uw-bothell-biology"}]}
            ]
        });
        let entries = get_review_queue_entries(&review_queue);
        assert_eq!(entries.len(), 3);
        assert_eq!(
            get_validation_scoped_review_queue_entries(&review_queue, Some("uw-seattle-biology"))
                .len(),
            2
        );
        let review_keys = build_review_owner_key_set(&review_queue);
        assert!(review_keys.contains("uw-seattle-biology"));
        assert!(review_keys.contains("uw-seattle-biology:pathway:general"));

        let discovery = json!({
            "owners": [
                {"ownerId": "one", "planId": "uw-seattle-biology"},
                {"ownerId": "two", "planId": "uw-bothell-biology"}
            ]
        });
        assert_eq!(
            get_validation_scoped_discovery_owners(
                &discovery,
                "owners",
                Some("uw-seattle-biology")
            )
            .len(),
            1
        );
        assert_eq!(
            get_validation_scoped_discovery_owners(&discovery, "owners", Some("")).len(),
            2
        );
    }

    #[test]
    fn review_owner_filters_and_schedulable_suggestion_rules_match_cjs() {
        assert!(is_structural_placeholder_review_owner(
            &json!({"pathwayId": "four-option"})
        ));
        assert!(is_structural_placeholder_review_owner(
            &json!({"ownerKey": "plan:PATHWAY:FOUR-OPTION"})
        ));
        let active_pathways = [json!({
            "id": "plan:pathway:active-route",
            "planId": "plan",
            "pathwayId": "active-route"
        })];
        assert!(!is_inactive_pathway_review_owner(
            &json!({"ownerType": "major", "ownerId": "plan"}),
            &active_pathways
        ));
        assert!(!is_inactive_pathway_review_owner(
            &json!({"ownerType": "pathway", "ownerId": "plan:pathway:active-route", "planId": "plan", "pathwayId": "active-route"}),
            &active_pathways
        ));
        assert!(is_inactive_pathway_review_owner(
            &json!({"ownerType": "pathway", "ownerId": "plan:pathway:retired-route", "planId": "plan", "pathwayId": "retired-route"}),
            &active_pathways
        ));

        assert!(is_schedulable_primary_suggestion(Some(
            &json!({"confidence": "high"})
        )));
        assert!(!is_schedulable_primary_suggestion(Some(&json!({
            "confidence": "high",
            "sourceRole": "curriculum-map"
        }))));
        assert!(!is_schedulable_primary_suggestion(Some(&json!({
            "confidence": "high",
            "canCreateSchedulableRows": false
        }))));
        assert!(current_primary_cannot_create_schedulable_rows(&json!({
            "currentPrimary": {"sourceRoleStatus": "support"}
        })));
        assert!(!current_primary_cannot_create_schedulable_rows(&json!({
            "currentPrimary": null
        })));
    }

    #[test]
    fn parser_backed_keys_canonicalize_pathway_ids_and_exclude_support_blocks() {
        let plan_id = "uw-seattle-biology";
        let active_pathways = [json!({
            "id": "uw-seattle-biology:pathway:bs-option-family:general-biology",
            "planId": plan_id,
            "pathwayId": "bs-option-family-general",
            "label": "General Biology"
        })];
        let parsed_blocks = [
            json!({
                "ok": true,
                "ownerId": "uw-seattle-biology:pathway:bs-option-family-general",
                "planId": plan_id,
                "pathwayId": "bs-option-family-general",
                "sourceRole": "primary-degree-requirements"
            }),
            json!({
                "ok": true,
                "ownerId": "uw-seattle-biology:pathway:general-biology-support",
                "planId": plan_id,
                "pathwayId": "general-biology-support",
                "sourceRole": "support-source"
            }),
        ];

        let keys = build_parser_backed_review_owner_keys(&parsed_blocks, &active_pathways);
        assert!(keys.contains(plan_id));
        assert!(keys.contains("uw-seattle-biology:pathway:bs-option-family:general-biology"));
        assert!(!keys.contains("uw-seattle-biology:pathway:general-biology-support"));
    }

    #[test]
    fn parser_backed_keys_infer_active_pathways_from_labels_and_source_text() {
        let plan_id = "uw-seattle-computer-science";
        let active_pathways = [
            json!({
                "id": "uw-seattle-computer-science:pathway:computer-science-option",
                "planId": plan_id,
                "pathwayId": "computer-science-option",
                "label": "Computer Science"
            }),
            json!({
                "id": "uw-seattle-computer-science:pathway:computer-engineering-track",
                "planId": plan_id,
                "pathwayId": "computer-engineering-track",
                "label": "Computer Engineering"
            }),
        ];
        let parsed_blocks = [
            json!({
                "ok": true,
                "planId": plan_id,
                "pathwayId": null,
                "pathwayLabels": ["Computer Science Option"]
            }),
            json!({
                "ok": true,
                "planId": plan_id,
                "pathwayId": null,
                "sourceLabel": "Computer Engineering Track Requirements"
            }),
        ];

        let keys = build_parser_backed_review_owner_keys(&parsed_blocks, &active_pathways);
        assert!(keys.contains("uw-seattle-computer-science:pathway:computer-science-option"));
        assert!(keys.contains("uw-seattle-computer-science:pathway:computer-engineering-track"));
    }

    #[test]
    fn successful_canonical_owner_covers_pathway_alias_for_same_plan_and_source() {
        let source = json!({
            "ownerId": "uw-seattle-example:pathway:sample-option",
            "planId": "uw-seattle-example",
            "pathwayId": "sample-option",
            "url": "https://example.edu/sample-option/contact"
        });
        let failed_owner = json!({
            "ownerId": "uw-seattle-example:pathway:sample-option",
            "planId": "uw-seattle-example",
            "pathwayId": "sample-option",
            "primarySourceUrl": "https://example.edu/sample-option/contact",
            "sourceUrl": "https://example.edu/sample-option/contact",
            "ok": false
        });
        let canonical_success = json!({
            "ownerId": "uw-seattle-example:pathway:canonical-owner",
            "planId": "uw-seattle-example",
            "primarySourceUrl": "https://example.edu/sample-option/contact",
            "ok": true
        });

        assert_eq!(
            find_failed_parseable_requirement_sources(
                std::slice::from_ref(&source),
                std::slice::from_ref(&failed_owner)
            ),
            [source.clone()]
        );
        assert!(
            find_failed_parseable_requirement_sources(
                std::slice::from_ref(&source),
                &[json!({
                    "ownerId": "uw-seattle-example:pathway:sample-option",
                    "primarySourceUrl": "https://example.edu/sample-option/contact",
                    "sourceUrl": "https://example.edu/sample-option/contact",
                    "ok": true
                })]
            )
            .is_empty()
        );
        assert!(
            find_failed_parseable_requirement_sources(
                std::slice::from_ref(&source),
                std::slice::from_ref(&canonical_success)
            )
            .is_empty()
        );
    }

    #[test]
    fn source_url_keys_follow_scheme_query_fragment_and_trailing_slash_rules() {
        let first = json!("http://astro.washington.edu/degree/path///?x=1#section");
        let second = json!("https://astro.washington.edu/degree/path");
        assert_eq!(
            normalize_requirement_source_url_for_coverage(Some(&first)),
            normalize_requirement_source_url_for_coverage(Some(&second))
        );
    }

    #[test]
    fn source_url_keys_canonicalize_tacoma_legacy_routes() {
        let old = json!("https://www.tacoma.uw.edu/sias-new/socs-new/general-option/?view=1#top");
        assert_eq!(
            normalize_requirement_source_url_for_coverage(Some(&old)),
            "https://www.tacoma.uw.edu/sias/socs/general-history-option"
        );
    }

    #[test]
    fn invalid_urls_use_javascript_trim_and_suffix_removal() {
        let value = json!("\u{FEFF}  NOT A URL/??cache=1 \u{FEFF}");
        assert_eq!(
            normalize_requirement_source_url_for_coverage(Some(&value)),
            "not a url"
        );
        let nel = json!("\u{0085}not a url\u{0085}");
        assert_eq!(
            normalize_requirement_source_url_for_coverage(Some(&nel)),
            "\u{0085}not a url\u{0085}"
        );
    }
}
