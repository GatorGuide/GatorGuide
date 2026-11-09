use chrono::{SecondsFormat, Utc};
use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::sync::OnceLock;
use url::Url;

pub fn build_parse_report(owners: &[Value], options: &Value) -> Value {
    let target_ids_value = options
        .get("targetPlanIds")
        .filter(|value| !value.is_null());
    let target_plan_ids = if let Some(values) = target_ids_value.and_then(Value::as_array) {
        unique_sorted_values(values.clone())
    } else if target_ids_value.is_some() {
        Vec::new()
    } else {
        options
            .get("targetPlanId")
            .filter(|value| js_truthy(value))
            .map(|value| unique_sorted_values(vec![value.clone()]))
            .unwrap_or_default()
    };
    let target_plan_id_set = (!target_plan_ids.is_empty()).then(|| target_plan_ids.clone());
    let mut merged_input = Vec::new();
    if let Some(target_set) = &target_plan_id_set {
        merged_input.extend(
            options
                .get("previousOwners")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|owner| {
                    !owner
                        .get("planId")
                        .is_some_and(|plan_id| target_set.contains(plan_id))
                })
                .cloned(),
        );
    }
    merged_input.extend_from_slice(owners);
    let mut full_owners = dedupe_parsed_owners_by_canonical_source(merged_input);
    sort_parsed_owners(&mut full_owners);
    full_owners = normalize_parsed_owners_for_report(&full_owners);
    let scoped_owners = if let Some(target_set) = &target_plan_id_set {
        full_owners
            .iter()
            .filter(|owner| {
                owner
                    .get("planId")
                    .is_some_and(|plan_id| target_set.contains(plan_id))
            })
            .cloned()
            .collect::<Vec<_>>()
    } else {
        full_owners.clone()
    };
    let signal_values = full_owners
        .iter()
        .flat_map(|owner| array_field(owner, "qualitySignals").iter().cloned())
        .collect::<Vec<_>>();
    let quality_warning_count = signal_values
        .iter()
        .filter(|signal| signal.get("severity").and_then(Value::as_str) == Some("warning"))
        .count();
    let quality_note_count = signal_values
        .iter()
        .filter(|signal| signal.get("severity").and_then(Value::as_str) == Some("note"))
        .count();
    let mut counts_by_quality_signal_code = Vec::new();
    for signal in &signal_values {
        counts_by_quality_signal_code.push(
            signal
                .get("code")
                .map(js_property_key)
                .unwrap_or_else(|| "undefined".to_string()),
        );
    }
    let count = |predicate: &dyn Fn(&Value) -> bool| {
        full_owners.iter().filter(|owner| predicate(owner)).count()
    };
    let target_plan_id = if target_plan_ids.len() == 1 {
        target_plan_ids[0].clone()
    } else {
        Value::Null
    };
    let mut report = Map::new();
    report.insert("generatedAt".to_string(), json!(now_iso()));
    report.insert("totalOwners".to_string(), json!(full_owners.len()));
    report.insert(
        "okCount".to_string(),
        json!(count(&|owner| owner.get("ok").is_some_and(js_truthy))),
    );
    report.insert(
        "failedCount".to_string(),
        json!(count(&|owner| !owner.get("ok").is_some_and(js_truthy))),
    );
    report.insert(
        "parsedRequirementSourceBlockCount".to_string(),
        json!(full_owners.len()),
    );
    report.insert(
        "parsedRequirementAtomCandidateCount".to_string(),
        json!(sum_array_lengths(
            &full_owners,
            "parsedRequirementAtomCandidates"
        )),
    );
    report.insert(
        "parsedDegreeMapBlockCandidateCount".to_string(),
        json!(sum_array_lengths(
            &full_owners,
            "parsedDegreeMapBlockCandidates"
        )),
    );
    report.insert(
        "parsedRequirementCourseCount".to_string(),
        json!(sum_array_lengths(&full_owners, "parsedRequirementCourses")),
    );
    report.insert(
        "snapshotFallbackCount".to_string(),
        json!(count(&|owner| owner
            .get("usedSnapshotFallback")
            .is_some_and(js_truthy))),
    );
    report.insert(
        "countsByAdapterId".to_string(),
        count_by_values(full_owners.iter().map(|owner| {
            owner
                .get("adapterId")
                .map(js_property_key)
                .unwrap_or_else(|| "undefined".to_string())
        })),
    );
    report.insert(
        "countsByAdapterFamily".to_string(),
        count_by_values(full_owners.iter().map(|owner| {
            owner
                .get("adapterFamily")
                .map(js_property_key)
                .unwrap_or_else(|| "undefined".to_string())
        })),
    );
    report.insert(
        "countsByCampus".to_string(),
        count_by_values(full_owners.iter().map(|owner| {
            owner
                .get("campusId")
                .map(js_property_key)
                .unwrap_or_else(|| "undefined".to_string())
        })),
    );
    report.insert(
        "countsByResolutionStrategy".to_string(),
        count_by_values(full_owners.iter().map(|owner| {
            owner
                .get("resolutionStrategy")
                .map(js_property_key)
                .unwrap_or_else(|| "undefined".to_string())
        })),
    );
    report.insert(
        "countsBySourceRole".to_string(),
        count_by_values(full_owners.iter().map(|owner| {
            owner
                .get("sourceRole")
                .filter(|value| !value.is_null())
                .map(js_property_key)
                .unwrap_or_else(|| "ignored".to_string())
        })),
    );
    report.insert(
        "countsBySourceRoleStatus".to_string(),
        count_by_values(full_owners.iter().map(|owner| {
            owner
                .get("sourceRoleStatus")
                .filter(|value| !value.is_null())
                .map(js_property_key)
                .unwrap_or_else(|| source_role_status(owner))
        })),
    );
    report.insert(
        "canCreateSchedulableRowCount".to_string(),
        json!(count(&|owner| owner
            .get("canCreateSchedulableRows")
            .and_then(Value::as_bool)
            != Some(false))),
    );
    report.insert(
        "canCreateRequiredRowCount".to_string(),
        json!(count(&|owner| owner
            .get("canCreateRequiredRows")
            .and_then(Value::as_bool)
            == Some(true))),
    );
    report.insert(
        "canCreateOptionGroupCount".to_string(),
        json!(count(&|owner| owner
            .get("canCreateOptionGroups")
            .and_then(Value::as_bool)
            == Some(true))),
    );
    report.insert(
        "canCreateApprovedFilterCount".to_string(),
        json!(count(&|owner| owner
            .get("canCreateApprovedFilters")
            .and_then(Value::as_bool)
            == Some(true))),
    );
    report.insert(
        "canCreateElectiveListCount".to_string(),
        json!(count(&|owner| owner
            .get("canCreateElectiveLists")
            .and_then(Value::as_bool)
            == Some(true))),
    );
    report.insert(
        "supportOnlySourceCount".to_string(),
        json!(count(&|owner| owner
            .get("supportOnly")
            .and_then(Value::as_bool)
            == Some(true))),
    );
    report.insert(
        "nonSchedulableSourceCount".to_string(),
        json!(count(&|owner| owner
            .get("nonSchedulable")
            .and_then(Value::as_bool)
            == Some(true))),
    );
    report.insert(
        "withParsedCourseCodesCount".to_string(),
        json!(count(
            &|owner| !array_field(owner, "parsedUwCourseCodes").is_empty()
        )),
    );
    report.insert(
        "withSourceOnlyCourseCodesCount".to_string(),
        json!(count(&|owner| !array_field(
            owner,
            "sourceOnlyUwCourseCodes"
        )
        .is_empty())),
    );
    report.insert(
        "withNoParsedCourseCodesCount".to_string(),
        json!(count(&|owner| {
            parsed_owner_has_actionable_no_parsed_courses(owner, &full_owners)
        })),
    );
    report.insert(
        "ownersWithQualityWarningsCount".to_string(),
        json!(count(&|owner| array_field(owner, "qualitySignals")
            .iter()
            .any(
                |signal| signal.get("severity").and_then(Value::as_str) == Some("warning")
            ))),
    );
    report.insert(
        "ownersWithQualityNotesCount".to_string(),
        json!(count(&|owner| array_field(owner, "qualitySignals")
            .iter()
            .any(
                |signal| signal.get("severity").and_then(Value::as_str) == Some("note")
            ))),
    );
    report.insert(
        "qualityWarningCount".to_string(),
        json!(quality_warning_count),
    );
    report.insert("qualityNoteCount".to_string(), json!(quality_note_count));
    report.insert(
        "countsByQualitySignalCode".to_string(),
        count_by_values(counts_by_quality_signal_code),
    );
    report.insert("targetPlanId".to_string(), target_plan_id);
    report.insert("targetPlanIds".to_string(), json!(target_plan_ids));
    report.insert("targetOwnerCount".to_string(), json!(scoped_owners.len()));
    report.insert(
        "sourceSectionAuditLines".to_string(),
        json!(
            scoped_owners
                .iter()
                .filter_map(|owner| owner
                    .get("sourceSectionAudit")
                    .and_then(|audit| audit.get("line"))
                    .filter(|line| js_truthy(line))
                    .cloned())
                .collect::<Vec<_>>()
        ),
    );
    report.insert(
        "sourceSectionFilterAuditLines".to_string(),
        json!(flatten_array_field(
            &scoped_owners,
            "sourceSectionFilterAuditLines"
        )),
    );
    report.insert(
        "parserSequenceChoiceAuditLines".to_string(),
        json!(flatten_array_field(
            &scoped_owners,
            "parserSequenceChoiceAuditLines"
        )),
    );
    report.insert(
        "sourceScopeAuditLines".to_string(),
        json!(flatten_array_field(&scoped_owners, "sourceScopeAuditLines")),
    );
    report.insert(
        "uwMseCourseExtractionAudit".to_string(),
        super::requirement_reports::build_uw_mse_course_extraction_audit(&full_owners),
    );
    report.insert(
        "parserRecoveryReport".to_string(),
        super::requirement_reports::build_parser_recovery_report(&full_owners),
    );
    report.insert("owners".to_string(), json!(full_owners.clone()));
    Value::Object(report)
}

fn normalize_parsed_owners_for_report(owners: &[Value]) -> Vec<Value> {
    owners
        .iter()
        .enumerate()
        .map(|(index, owner)| {
            let mut normalized = owner.clone();
            if parsed_owner_is_covered_by_child_pathway_requirements(index, owner, owners) {
                normalized = remove_quality_signal(&normalized, "no-parsed-uw-course-codes");
            }
            if parsed_owner_structured_only_codes_covered_by_same_plan_sources(
                index,
                &normalized,
                owners,
            ) {
                normalized = remove_quality_signals(
                    &normalized,
                    &[
                        "material-source-structured-drift",
                        "large-structured-only-course-gap",
                        "high-confidence-low-course-coverage",
                    ],
                );
            }
            normalized
        })
        .collect()
}

fn parsed_owner_can_create_schedulable_rows(owner: &Value) -> bool {
    owner
        .get("canCreateSchedulableRows")
        .and_then(Value::as_bool)
        != Some(false)
        && owner.get("supportOnly").and_then(Value::as_bool) != Some(true)
        && owner.get("nonSchedulable").and_then(Value::as_bool) != Some(true)
        && source_role_status(owner) == "primary"
}

fn parsed_owner_is_covered_by_child_pathway_requirements(
    index: usize,
    owner: &Value,
    owners: &[Value],
) -> bool {
    if !owner.get("ok").is_some_and(js_truthy)
        || !array_field(owner, "parsedUwCourseCodes").is_empty()
    {
        return false;
    }
    let owner_key = parsed_owner_dedupe_key(owner);
    let plan_id = owner.get("planId");
    let owner_id = owner.get("ownerId");
    let has_pathway = owner.get("pathwayId").is_some_and(js_truthy);
    let covering = owners
        .iter()
        .enumerate()
        .any(|(candidate_index, candidate)| {
            if candidate_index == index
                || candidate.get("planId") != plan_id
                || owner_key
                    .as_ref()
                    .is_some_and(|key| parsed_owner_dedupe_key(candidate).as_ref() == Some(key))
                || !candidate.get("ok").is_some_and(js_truthy)
                || !parsed_owner_can_create_schedulable_rows(candidate)
                || array_field(candidate, "parsedUwCourseCodes").is_empty()
            {
                return false;
            }
            candidate.get("ownerId") == owner_id
                || (!has_pathway && candidate.get("pathwayId").is_some_and(js_truthy))
        });
    if !covering {
        return false;
    }
    let mut overview_parts = Vec::new();
    for key in [
        "primaryParserType",
        "parserType",
        "primarySourceLabel",
        "sourceLabel",
        "primarySourceUrl",
        "sourceUrl",
    ] {
        if let Some(value) = owner.get(key).filter(|value| js_truthy(value)) {
            overview_parts.push(js_string(value));
        }
    }
    for key in ["pathwayLabels", "requirementCueLines"] {
        overview_parts.extend(
            array_field(owner, key)
                .iter()
                .filter(|value| js_truthy(value))
                .map(js_string),
        );
    }
    let overview_text = super::requirement_recovery_links::normalize_requirement_matcher_text(
        &overview_parts.join(" "),
    );
    !array_field(owner, "pathwayLabels").is_empty() || overview_pattern().is_match(&overview_text)
}

fn parsed_owner_has_actionable_no_parsed_courses(owner: &Value, owners: &[Value]) -> bool {
    owner.get("ok").is_some_and(js_truthy)
        && array_field(owner, "parsedUwCourseCodes").is_empty()
        && parsed_owner_can_create_schedulable_rows(owner)
        && !owners.iter().enumerate().any(|(index, candidate)| {
            std::ptr::eq(candidate, owner)
                && parsed_owner_is_covered_by_child_pathway_requirements(index, candidate, owners)
        })
}

fn parsed_owner_structured_only_codes_covered_by_same_plan_sources(
    index: usize,
    owner: &Value,
    owners: &[Value],
) -> bool {
    let structured_codes = array_field(owner, "structuredOnlyUwCourseCodes")
        .iter()
        .filter_map(Value::as_str)
        .map(super::requirement_course_codes::normalize_course_code)
        .filter(|code| !code.is_empty())
        .collect::<Vec<_>>();
    if !owner.get("ok").is_some_and(js_truthy)
        || structured_codes.is_empty()
        || !owner.get("planId").is_some_and(js_truthy)
    {
        return false;
    }
    let owner_key = parsed_owner_dedupe_key(owner);
    let mut parsed_codes = HashSet::new();
    for (candidate_index, candidate) in owners.iter().enumerate() {
        if candidate_index == index
            || owner_key
                .as_ref()
                .is_some_and(|key| parsed_owner_dedupe_key(candidate).as_ref() == Some(key))
            || candidate.get("planId") != owner.get("planId")
            || !candidate.get("ok").is_some_and(js_truthy)
            || !parsed_owner_can_create_schedulable_rows(candidate)
        {
            continue;
        }
        parsed_codes.extend(
            array_field(candidate, "parsedUwCourseCodes")
                .iter()
                .filter_map(Value::as_str)
                .map(super::requirement_course_codes::normalize_course_code)
                .filter(|code| !code.is_empty()),
        );
    }
    !parsed_codes.is_empty()
        && structured_codes
            .iter()
            .all(|code| parsed_codes.contains(code))
}

fn remove_quality_signal(owner: &Value, code: &str) -> Value {
    remove_quality_signals(owner, &[code])
}

fn remove_quality_signals(owner: &Value, codes: &[&str]) -> Value {
    let existing = array_field(owner, "qualitySignals");
    let filtered = existing
        .iter()
        .filter(|signal| {
            !signal
                .get("code")
                .and_then(Value::as_str)
                .is_some_and(|code| codes.contains(&code))
        })
        .cloned()
        .collect::<Vec<_>>();
    if filtered.len() == existing.len() {
        return owner.clone();
    }
    let mut result = owner.as_object().cloned().unwrap_or_default();
    result.insert("qualitySignals".to_string(), json!(filtered));
    Value::Object(result)
}

fn dedupe_parsed_owners_by_canonical_source(owners: Vec<Value>) -> Vec<Value> {
    let mut keyed = Vec::<(String, Value)>::new();
    let mut unkeyed = Vec::new();
    for owner in owners {
        let Some(key) = parsed_owner_dedupe_key(&owner) else {
            unkeyed.push(owner);
            continue;
        };
        if let Some(index) = keyed
            .iter()
            .position(|(current_key, _)| current_key == &key)
        {
            let merged = merge_parsed_owner_canonical_source_coverage(&keyed[index].1, &owner);
            keyed[index].1 = merged;
        } else {
            let mut first = owner.clone();
            if let Some(fields) = first.as_object_mut() {
                fields.insert(
                    "coveredSourceUrls".to_string(),
                    json!(covered_source_urls(&owner)),
                );
            }
            keyed.push((key, first));
        }
    }
    keyed
        .into_iter()
        .map(|(_, owner)| owner)
        .chain(unkeyed)
        .collect()
}

fn merge_parsed_owner_canonical_source_coverage(left: &Value, right: &Value) -> Value {
    let selected = if parsed_owner_dedupe_score(left) >= parsed_owner_dedupe_score(right) {
        left
    } else {
        right
    };
    let mut result = selected.as_object().cloned().unwrap_or_default();
    result.insert(
        "coveredSourceUrls".to_string(),
        json!(unique_sorted_values(
            [covered_source_urls(left), covered_source_urls(right)].concat()
        )),
    );
    for field in [
        "parsedUwCourseCodes",
        "approvedFilterUwCourseCodes",
        "electiveListUwCourseCodes",
        "supportOnlyUwCourseCodes",
        "sourceOnlyUwCourseCodes",
        "structuredOnlyUwCourseCodes",
    ] {
        result.insert(
            field.to_string(),
            json!(unique_sorted_values(
                [
                    array_field(left, field).to_vec(),
                    array_field(right, field).to_vec()
                ]
                .concat()
            )),
        );
    }
    for field in [
        "requirementCueLines",
        "chooseStatements",
        "pathwayLabels",
        "sourceScopeAuditLines",
    ] {
        result.insert(
            field.to_string(),
            json!(unique_in_order_values(
                [
                    array_field(left, field).to_vec(),
                    array_field(right, field).to_vec()
                ]
                .concat()
            )),
        );
    }
    result.insert(
        "supportLists".to_string(),
        json!(unique_by_values(
            [
                array_field(left, "supportLists").to_vec(),
                array_field(right, "supportLists").to_vec()
            ]
            .concat(),
            support_list_semantic_key
        )),
    );
    result.insert(
        "parsedRequirementAtomCandidates".to_string(),
        json!(unique_by_values(
            [
                array_field(left, "parsedRequirementAtomCandidates").to_vec(),
                array_field(right, "parsedRequirementAtomCandidates").to_vec()
            ]
            .concat(),
            |value| {
                nullish_string(value.get("id")).unwrap_or_else(|| {
                    format!(
                        "{}:{}",
                        nullish_string(value.get("courseCode")).unwrap_or_default(),
                        nullish_string(value.get("sourceRowText")).unwrap_or_default()
                    )
                })
            }
        )),
    );
    result.insert(
        "parsedDegreeMapBlockCandidates".to_string(),
        json!(unique_by_values(
            [
                array_field(left, "parsedDegreeMapBlockCandidates").to_vec(),
                array_field(right, "parsedDegreeMapBlockCandidates").to_vec()
            ]
            .concat(),
            |value| {
                nullish_string(value.get("id")).unwrap_or_else(|| {
                    format!(
                        "{}:{}",
                        nullish_string(value.get("title")).unwrap_or_default(),
                        array_field(value, "uwCourseCodes")
                            .iter()
                            .map(js_string)
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                })
            }
        )),
    );
    result.insert(
        "parsedRequirementGroups".to_string(),
        json!(unique_by_values(
            [
                array_field(left, "parsedRequirementGroups").to_vec(),
                array_field(right, "parsedRequirementGroups").to_vec()
            ]
            .concat(),
            |value| {
                nullish_string(value.get("id")).unwrap_or_else(|| {
                    format!(
                        "{}:{}",
                        nullish_string(value.get("label")).unwrap_or_default(),
                        nullish_string(value.get("requirementType")).unwrap_or_default()
                    )
                })
            }
        )),
    );
    result.insert(
        "parsedRequirementCourses".to_string(),
        json!(unique_by_values(
            [
                array_field(left, "parsedRequirementCourses").to_vec(),
                array_field(right, "parsedRequirementCourses").to_vec()
            ]
            .concat(),
            |value| {
                nullish_string(value.get("id")).unwrap_or_else(|| {
                    format!(
                        "{}:{}",
                        nullish_string(value.get("normalizedCourseCode")).unwrap_or_default(),
                        nullish_string(value.get("label")).unwrap_or_default()
                    )
                })
            }
        )),
    );
    for (rows_field, lines_field) in [
        (
            "sourceSectionFilterAuditRows",
            "sourceSectionFilterAuditLines",
        ),
        (
            "parserSequenceChoiceAuditRows",
            "parserSequenceChoiceAuditLines",
        ),
    ] {
        let rows = [
            array_field(left, rows_field).to_vec(),
            array_field(right, rows_field).to_vec(),
        ]
        .concat();
        let lines = rows
            .iter()
            .map(|row| row.get("copyOnlyDebugText").cloned().unwrap_or(Value::Null))
            .collect::<Vec<_>>();
        result.insert(rows_field.to_string(), json!(rows));
        result.insert(lines_field.to_string(), json!(lines));
    }
    Value::Object(result)
}

fn parsed_owner_dedupe_key(owner: &Value) -> Option<String> {
    let owner_id = owner
        .get("ownerId")
        .filter(|value| !value.is_null())
        .map(js_string)
        .unwrap_or_default()
        .trim_matches(super::requirement_shape::is_js_whitespace)
        .to_string();
    let source_url = normalize_url_for_comparison(owner.get("sourceUrl"))?;
    (!owner_id.is_empty()).then(|| format!("{owner_id} {source_url}"))
}

fn parsed_owner_dedupe_score(owner: &Value) -> i64 {
    let mut score = 0;
    if owner.get("ok").is_some_and(js_truthy) {
        score += 1000;
    }
    if owner
        .get("isPrimaryDegreeRequirementsLink")
        .is_some_and(js_truthy)
    {
        score += 300;
    }
    let source_url = normalize_url_for_comparison(owner.get("sourceUrl"));
    let primary_url = normalize_url_for_comparison(owner.get("primarySourceUrl"));
    if source_url.is_some() && source_url == primary_url {
        score += 250;
    }
    if owner.get("resolutionStrategy").and_then(Value::as_str) == Some("primary-source") {
        score += 100;
    }
    match source_role_status(owner).as_str() {
        "primary" => score += 50,
        "support" => score += 20,
        _ => {}
    }
    score += array_field(owner, "parsedRequirementCourses")
        .len()
        .min(100) as i64;
    score += (array_field(owner, "parsedRequirementGroups").len() * 2).min(50) as i64;
    score -= array_field(owner, "qualitySignals")
        .iter()
        .filter(|signal| signal.get("severity").and_then(Value::as_str) == Some("warning"))
        .count() as i64
        * 10;
    score
}

fn covered_source_urls(owner: &Value) -> Vec<Value> {
    let mut urls = array_field(owner, "coveredSourceUrls").to_vec();
    urls.extend([
        owner
            .get("primarySourceUrl")
            .cloned()
            .unwrap_or(Value::Null),
        owner.get("sourceUrl").cloned().unwrap_or(Value::Null),
    ]);
    unique_sorted_values(urls)
}

fn support_list_semantic_key(list: &Value) -> String {
    let shape = nullish_string(list.get("shape")).unwrap_or_default();
    let source_url = nullish_string(list.get("sourceUrl")).unwrap_or_default();
    let approved_key = nullish_string(
        list.get("approvedListKey")
            .filter(|value| !value.is_null())
            .or_else(|| list.get("filterKey")),
    )
    .unwrap_or_default();
    if !approved_key.is_empty()
        && ["approved-filter-list", "approved-course-list"].contains(&shape.as_str())
    {
        format!("approved:{shape}:{approved_key}")
    } else if let Some(id) = list.get("id").filter(|value| js_truthy(value)) {
        js_string(id)
    } else {
        format!(
            "{shape}:{source_url}:{}",
            nullish_string(list.get("listTitle")).unwrap_or_default()
        )
    }
}

fn unique_by_values<F>(values: Vec<Value>, key_fn: F) -> Vec<Value>
where
    F: Fn(&Value) -> String,
{
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| {
            let key = key_fn(value);
            !key.is_empty() && seen.insert(key)
        })
        .collect()
}

fn unique_sorted_values(values: Vec<Value>) -> Vec<Value> {
    let mut unique = Vec::<Value>::new();
    for value in values.into_iter().filter(js_truthy) {
        if !unique.contains(&value) {
            unique.push(value);
        }
    }
    unique.sort_by(|left, right| {
        let left = js_string(left);
        let right = js_string(right);
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then_with(|| right.cmp(&left))
    });
    unique
}

fn unique_in_order_values(values: Vec<Value>) -> Vec<Value> {
    let mut unique = Vec::<Value>::new();
    for value in values.into_iter().filter(js_truthy) {
        if !unique.contains(&value) {
            unique.push(value);
        }
    }
    unique
}

fn sort_parsed_owners(owners: &mut [Value]) {
    owners.sort_by(|left, right| {
        let left_title = nullish_string(left.get("ownerTitle")).unwrap_or_default();
        let right_title = nullish_string(right.get("ownerTitle")).unwrap_or_default();
        let left_id = nullish_string(left.get("ownerId")).unwrap_or_default();
        let right_id = nullish_string(right.get("ownerId")).unwrap_or_default();
        let left_url = nullish_string(left.get("sourceUrl")).unwrap_or_default();
        let right_url = nullish_string(right.get("sourceUrl")).unwrap_or_default();
        locale_compare(&left_title, &right_title)
            .then_with(|| locale_compare(&left_id, &right_id))
            .then_with(|| locale_compare(&left_url, &right_url))
    });
}

fn locale_compare(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| right.cmp(left))
}

fn source_role_status(owner: &Value) -> String {
    if let Some(status) = owner
        .get("sourceRoleStatus")
        .filter(|value| !value.is_null())
    {
        return status.as_str().unwrap_or_default().to_string();
    }
    let role = owner
        .get("sourceRole")
        .filter(|value| !value.is_null())
        .map(js_string)
        .unwrap_or_else(|| "ignored".to_string());
    super::requirement_scope::get_requirement_source_role_status(Some(role.as_str()))
        .as_str()
        .to_string()
}

fn count_by_values(values: impl IntoIterator<Item = String>) -> Value {
    let mut counts = Map::new();
    for key in values {
        let count = counts.get(&key).and_then(Value::as_u64).unwrap_or(0) + 1;
        counts.insert(key, json!(count));
    }
    Value::Object(counts)
}

fn flatten_array_field(owners: &[Value], field: &str) -> Vec<Value> {
    owners
        .iter()
        .flat_map(|owner| array_field(owner, field).iter().cloned())
        .collect()
}

fn sum_array_lengths(owners: &[Value], field: &str) -> usize {
    owners
        .iter()
        .map(|owner| array_field(owner, field).len())
        .sum()
}

fn array_field<'a>(owner: &'a Value, field: &str) -> &'a [Value] {
    owner
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn normalize_url_for_comparison(value: Option<&Value>) -> Option<String> {
    let value = nullish_string(value)?;
    let mut parsed = Url::parse(&value).ok()?;
    parsed.set_fragment(None);
    let path = parsed.path().trim_end_matches('/');
    let path = if path.is_empty() { "/" } else { path };
    Some(format!(
        "{}{}{}",
        parsed.origin().ascii_serialization(),
        path,
        parsed
            .query()
            .map(|query| format!("?{query}"))
            .unwrap_or_default()
    ))
}

fn nullish_string(value: Option<&Value>) -> Option<String> {
    value.filter(|value| !value.is_null()).map(js_string)
}

fn js_property_key(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        _ => js_string(value),
    }
}

fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => super::requirement_shape::js_number_to_string(value),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(|value| {
                if value.is_null() {
                    String::new()
                } else {
                    js_string(value)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
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

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn overview_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        let pattern = super::requirement_shape::js_regex_compatibility_pattern(
            r"\b(?:overview|options?|tracks?|routes?|pathways?|concentrations?)\b",
        );
        Regex::new(&pattern).expect("overview matcher")
    })
}
