use super::requirement_course_codes::{CourseCodeParser, normalize_course_code};
use super::requirement_snapshots::RequirementSnapshotStore;
use chrono::{SecondsFormat, Utc};
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::OnceLock;

#[derive(Clone)]
struct FilterRecord {
    owner: Value,
    support_list: Value,
    filter_key: String,
}

pub fn build_generated_program_approved_course_filters(
    report: &Value,
    snapshot_store: &RequirementSnapshotStore,
    course_parser: &CourseCodeParser,
) -> Vec<Value> {
    let owners = report
        .get("owners")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut records = Vec::<FilterRecord>::new();
    for owner in owners {
        let support_lists = array_field(owner, "supportLists");
        for support_list in support_lists {
            let raw_key = support_list
                .get("approvedListKey")
                .filter(|value| !value.is_null())
                .or_else(|| support_list.get("filterKey"))
                .filter(|value| truthy(value));
            if let Some(raw_key) = raw_key {
                records.push(FilterRecord {
                    owner: (*owner).clone(),
                    support_list: (*support_list).clone(),
                    filter_key: normalize_filter_key(&js_string(raw_key)),
                });
            }
        }
        if owner.get("sourceRole").and_then(Value::as_str) == Some("approved-course-list")
            || owner
                .get("canCreateApprovedFilters")
                .and_then(Value::as_bool)
                == Some(true)
        {
            if let Some(inferred_key) =
                super::requirement_source_audits::infer_approved_list_key_from_support_metadata(
                    owner,
                )
            {
                let already_present = support_lists.iter().any(|list| {
                    list.get("approvedListKey").and_then(Value::as_str)
                        == Some(inferred_key.as_str())
                });
                if !already_present {
                    records.push(FilterRecord {
                        owner: (*owner).clone(),
                        support_list: json!({
                            "id": format!("{}:support-list:{inferred_key}", owner.get("ownerId").map(js_string).unwrap_or_else(|| "undefined".to_string())),
                            "shape": "approved-filter-list",
                            "sourceUrl": owner.get("sourceUrl").cloned().unwrap_or(Value::Null),
                            "sourceRole": owner.get("sourceRole").cloned().unwrap_or(Value::Null),
                            "listTitle": owner.get("sourceLabel").cloned().unwrap_or(Value::Null),
                            "approvedListKey": inferred_key.clone(),
                            "acceptedUwCourseCodes": [],
                        }),
                        filter_key: normalize_filter_key(&inferred_key),
                    });
                }
            }
        }
    }

    let mut grouped_records = Vec::<(String, Vec<FilterRecord>)>::new();
    for record in records {
        if record.filter_key.is_empty() {
            continue;
        }
        if let Some((_, group)) = grouped_records
            .iter_mut()
            .find(|(key, _)| key == &record.filter_key)
        {
            group.push(record);
        } else {
            grouped_records.push((record.filter_key.clone(), vec![record]));
        }
    }

    let generated_at = report
        .get("generatedAt")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| json!(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)));
    let mut filters = Vec::new();
    for (filter_key, key_records) in grouped_records {
        let Some(candidate) = find_filter_evidence_candidate(
            &filter_key,
            &key_records,
            owners,
            snapshot_store,
            course_parser,
        ) else {
            continue;
        };
        if array_field(&candidate.evidence, "approvedUwCourseCodes").is_empty() {
            continue;
        }
        filters.push(build_filter_definition(
            &filter_key,
            &key_records,
            &candidate,
            &generated_at,
        ));
    }
    if !filters.iter().any(|filter| {
        filter.get("filterKey").and_then(Value::as_str) == Some("computer-engineering-math-science")
    }) {
        if let Some(filter) = build_computer_engineering_math_science_filter(
            owners,
            &filters,
            &generated_at,
            snapshot_store,
            course_parser,
        ) {
            filters.push(filter);
        }
    }
    filters.sort_by(|left, right| {
        left.get("filterKey")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .cmp(
                right
                    .get("filterKey")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
    });
    filters
}

pub fn render_generated_program_approved_course_filters(
    filters: &[Value],
) -> Result<String, String> {
    let serialized = serde_json::to_string_pretty(filters)
        .map_err(|error| format!("could not serialize generated approved filters: {error}"))?;
    Ok(format!(
        "/* eslint-disable */\n/* auto-generated by scripts/planner/parse-transfer-planner-requirement-sources.cjs */\n\nexport const TRANSFER_PLANNER_GENERATED_PROGRAM_APPROVED_COURSE_FILTERS = {serialized} as const;\n"
    ))
}

pub fn build_generated_program_approved_course_filters_typescript(
    report: &Value,
    snapshot_store: &RequirementSnapshotStore,
    course_parser: &CourseCodeParser,
) -> Result<String, String> {
    let filters =
        build_generated_program_approved_course_filters(report, snapshot_store, course_parser);
    render_generated_program_approved_course_filters(&filters)
}

fn find_filter_evidence_candidate(
    filter_key: &str,
    records: &[FilterRecord],
    owners: &[Value],
    snapshot_store: &RequirementSnapshotStore,
    course_parser: &CourseCodeParser,
) -> Option<FilterEvidenceCandidate> {
    let fallback_codes = unique_sorted_strings(records.iter().flat_map(|record| {
        array_field(&record.support_list, "acceptedUwCourseCodes")
            .iter()
            .map(|code| normalize_course_code(&js_string(code)))
            .filter(|code| !code.is_empty())
    }));
    let record_owner_ids = records
        .iter()
        .filter_map(|record| {
            record
                .owner
                .get("ownerId")
                .filter(|value| truthy(value))
                .map(js_string)
        })
        .collect::<std::collections::HashSet<_>>();
    let support_owners = owners.iter().filter(|owner| {
        let has_matching = array_field(owner, "supportLists")
            .iter()
            .any(|list| list.get("approvedListKey").and_then(Value::as_str) == Some(filter_key));
        if !fallback_codes.is_empty() {
            has_matching
                || owner
                    .get("ownerId")
                    .filter(|value| truthy(value))
                    .is_some_and(|id| record_owner_ids.contains(&js_string(id)))
        } else {
            owner.get("sourceRole").and_then(Value::as_str) == Some("approved-course-list")
                || owner
                    .get("canCreateApprovedFilters")
                    .and_then(Value::as_bool)
                    == Some(true)
                || has_matching
        }
    });
    let mut candidates = Vec::<FilterEvidenceCandidate>::new();
    for owner in support_owners {
        let lines = snapshot_lines_for_owner(owner, snapshot_store);
        if lines.is_empty() {
            continue;
        }
        let input = json!({
            "filterKey": filter_key,
            "owner": owner,
            "supportList": records.first().map(|record| record.support_list.clone()).unwrap_or_else(|| json!({})),
            "lines": lines,
        });
        let Some(evidence) =
            super::requirement_source_audits::extract_approved_filter_evidence_from_lines(
                &input,
                course_parser,
            )
        else {
            continue;
        };
        if array_field(&evidence, "approvedUwCourseCodes").is_empty() {
            continue;
        }
        let owner_id = owner
            .get("ownerId")
            .filter(|value| truthy(value))
            .map(js_string);
        let score = array_field(&evidence, "approvedUwCourseCodes").len() as i64
            + (if owner_id
                .as_ref()
                .is_some_and(|id| record_owner_ids.contains(id))
            {
                100
            } else {
                0
            })
            + (if owner.get("sourceRole").and_then(Value::as_str) == Some("approved-course-list") {
                20
            } else {
                0
            });
        candidates.push(FilterEvidenceCandidate {
            owner: (*owner).clone(),
            evidence,
            score,
        });
    }
    if candidates.is_empty() && !fallback_codes.is_empty() {
        let first = records.first()?;
        let title = first
            .support_list
            .get("listTitle")
            .filter(|value| truthy(value))
            .or_else(|| first.owner.get("sourceLabel").filter(|value| truthy(value)))
            .cloned();
        let mut headings = Vec::new();
        let mut evidence_lines = Vec::new();
        if let Some(title) = title {
            headings.push(title.clone());
            evidence_lines.push(title);
        }
        let source_url = first
            .owner
            .get("sourceUrl")
            .filter(|value| truthy(value))
            .map(js_string)
            .unwrap_or_else(|| "official support source".to_string());
        evidence_lines.push(json!(format!(
            "Parsed approved course codes from {source_url}."
        )));
        return Some(FilterEvidenceCandidate {
            owner: first.owner.clone(),
            evidence: json!({
                "approvedUwCourseCodes": fallback_codes,
                "approvedUwCourseGroups": [],
                "petitionOnlyNotes": [],
                "sourceEvidenceHeadings": headings,
                "sourceEvidenceLines": evidence_lines,
            }),
            score: fallback_codes.len() as i64,
        });
    }
    if candidates.is_empty() {
        for snapshot in read_all_snapshots(snapshot_store) {
            let lines = snapshot.snapshot_lines;
            if lines.is_empty() {
                continue;
            }
            let first = records.first();
            let owner = first
                .map(|record| record.owner.clone())
                .unwrap_or_else(|| json!({}));
            let support_list = first
                .map(|record| record.support_list.clone())
                .unwrap_or_else(|| json!({}));
            let input = json!({"filterKey": filter_key, "owner": owner, "supportList": support_list, "lines": lines});
            let Some(evidence) =
                super::requirement_source_audits::extract_approved_filter_evidence_from_lines(
                    &input,
                    course_parser,
                )
            else {
                continue;
            };
            if array_field(&evidence, "approvedUwCourseCodes").is_empty() {
                continue;
            }
            let mut evidence_owner = first
                .map(|record| record.owner.clone())
                .unwrap_or_else(|| json!({}));
            if let Some(fields) = evidence_owner.as_object_mut() {
                if fields.get("sourceUrl").is_none_or(Value::is_null) {
                    if let Some(source_url) = snapshot.source_url.clone() {
                        fields.insert("sourceUrl".to_string(), json!(source_url));
                    }
                }
                if fields.get("sourceRole").is_none_or(Value::is_null) {
                    fields.insert("sourceRole".to_string(), json!("approved-course-list"));
                }
            }
            candidates.push(FilterEvidenceCandidate {
                owner: evidence_owner,
                score: array_field(&evidence, "approvedUwCourseCodes").len() as i64 + 10,
                evidence,
            });
        }
    }
    candidates.sort_by(|left, right| right.score.cmp(&left.score));
    candidates.into_iter().next()
}

#[derive(Clone)]
struct FilterEvidenceCandidate {
    owner: Value,
    evidence: Value,
    score: i64,
}

fn build_filter_definition(
    filter_key: &str,
    records: &[FilterRecord],
    candidate: &FilterEvidenceCandidate,
    generated_at: &Value,
) -> Value {
    let (filter_id, label, aliases, generic_tags, allow_additional) =
        compatibility_for_filter_key(filter_key);
    let owner_ids = unique_sorted_values(
        records
            .iter()
            .filter_map(|record| {
                record
                    .owner
                    .get("ownerId")
                    .filter(|value| truthy(value))
                    .cloned()
            })
            .collect(),
    );
    let campus_id = records
        .iter()
        .find_map(|record| {
            record
                .owner
                .get("campusId")
                .filter(|value| truthy(value))
                .cloned()
        })
        .or_else(|| {
            candidate
                .owner
                .get("campusId")
                .filter(|value| truthy(value))
                .cloned()
        })
        .unwrap_or_else(|| json!("uw-seattle"));
    let official_source_url = preferred_official_source_url(records)
        .or_else(|| {
            candidate
                .owner
                .get("sourceUrl")
                .filter(|value| truthy(value))
                .cloned()
        })
        .unwrap_or_else(|| json!(""));
    let source_url = candidate
        .owner
        .get("sourceUrl")
        .filter(|value| truthy(value))
        .cloned()
        .unwrap_or_else(|| official_source_url.clone());
    let source_role = candidate
        .owner
        .get("sourceRole")
        .filter(|value| !value.is_null())
        .cloned()
        .or_else(|| {
            records
                .first()
                .and_then(|record| record.owner.get("sourceRole").cloned())
        })
        .unwrap_or(Value::Null);
    let mut codes = array_field(&candidate.evidence, "approvedUwCourseCodes")
        .iter()
        .map(|code| normalize_course_code(&js_string(code)))
        .collect::<Vec<_>>();
    codes.extend(records.iter().flat_map(|record| {
        array_field(&record.support_list, "acceptedUwCourseCodes")
            .iter()
            .map(|code| normalize_course_code(&js_string(code)))
    }));
    let codes = json!(unique_sorted_strings(
        codes.into_iter().filter(|code| !code.is_empty())
    ));
    let groups = array_field(&candidate.evidence, "approvedUwCourseGroups");
    let notes = array_field(&candidate.evidence, "petitionOnlyNotes");
    let evidence_headings = array_field(&candidate.evidence, "sourceEvidenceHeadings");
    let evidence_lines = array_field(&candidate.evidence, "sourceEvidenceLines");
    let mut filter = Map::new();
    filter.insert("filterId".to_string(), json!(filter_id));
    filter.insert("filterKey".to_string(), json!(filter_key));
    filter.insert("label".to_string(), json!(label));
    filter.insert("ownerIds".to_string(), owner_ids);
    filter.insert("campusId".to_string(), campus_id);
    filter.insert("officialSourceUrl".to_string(), official_source_url);
    filter.insert("sourceUrl".to_string(), source_url);
    filter.insert("sourceRole".to_string(), source_role);
    filter.insert("approvedUwCourseCodes".to_string(), codes);
    if !groups.is_empty() {
        filter.insert("approvedUwCourseGroups".to_string(), json!(groups));
    }
    if !notes.is_empty() {
        filter.insert("petitionOnlyNotes".to_string(), json!(notes));
    }
    if !generic_tags.is_empty() {
        filter.insert("genericCategoryTags".to_string(), json!(generic_tags));
    }
    let aliases = unique_in_order_strings(std::iter::once(filter_key.to_string()).chain(aliases));
    filter.insert("aliases".to_string(), json!(aliases));
    if allow_additional {
        filter.insert(
            "allowEquivalencyRulesWithAdditionalTargets".to_string(),
            json!(true),
        );
    }
    filter.insert(
        "generatedFromOfficialSupportSource".to_string(),
        json!(true),
    );
    filter.insert(
        "sourceEvidenceHeadings".to_string(),
        json!(unique_in_order_values(evidence_headings.to_vec())),
    );
    filter.insert(
        "sourceEvidenceLines".to_string(),
        json!(unique_in_order_values(evidence_lines.to_vec())),
    );
    filter.insert("generatedAt".to_string(), generated_at.clone());
    let mut fingerprint_input = Map::new();
    for key in [
        "filterKey",
        "ownerIds",
        "officialSourceUrl",
        "sourceUrl",
        "sourceRole",
        "approvedUwCourseCodes",
    ] {
        if let Some(value) = filter.get(key) {
            fingerprint_input.insert(key.to_string(), value.clone());
        }
    }
    fingerprint_input.insert(
        "approvedUwCourseGroups".to_string(),
        groups
            .first()
            .map(|_| json!(groups))
            .unwrap_or_else(|| json!([])),
    );
    fingerprint_input.insert(
        "petitionOnlyNotes".to_string(),
        notes
            .first()
            .map(|_| json!(notes))
            .unwrap_or_else(|| json!([])),
    );
    fingerprint_input.insert(
        "sourceEvidenceLines".to_string(),
        json!(unique_in_order_values(evidence_lines.to_vec())),
    );
    let fingerprint = fingerprint_json(&Value::Object(fingerprint_input));
    filter.insert("sourceFingerprint".to_string(), json!(fingerprint));
    Value::Object(filter)
}

fn build_computer_engineering_math_science_filter(
    owners: &[Value],
    generated_filters: &[Value],
    generated_at: &Value,
    snapshot_store: &RequirementSnapshotStore,
    course_parser: &CourseCodeParser,
) -> Option<Value> {
    let natural_science_filter = generated_filters.iter().find(|filter| {
        filter.get("filterKey").and_then(Value::as_str)
            == Some("computer-engineering-natural-science")
    })?;
    let owner = owners.iter().find(|owner| {
        owner.get("planId").and_then(Value::as_str) == Some("uw-seattle-computer-engineering")
            && array_field(owner, "parsedRequirementGroups")
                .iter()
                .any(|group| {
                    group.get("approvedListKey").and_then(Value::as_str)
                        == Some("computer-engineering-math-science")
                })
    })?;
    let (extra_codes, extra_lines) =
        extract_computer_engineering_math_science_extra_codes(owner, snapshot_store, course_parser);
    let (filter_id, label, aliases, generic_tags, _) =
        compatibility_for_filter_key("computer-engineering-math-science");
    let mut codes = array_field(natural_science_filter, "approvedUwCourseCodes")
        .iter()
        .map(|code| normalize_course_code(&js_string(code)))
        .filter(|code| !code.is_empty())
        .collect::<Vec<_>>();
    codes.extend(extra_codes);
    let mut headings = vec![json!("Computer Engineering Math/Science bucket")];
    headings.extend(
        array_field(natural_science_filter, "sourceEvidenceHeadings")
            .iter()
            .cloned(),
    );
    let mut evidence_lines = extra_lines
        .into_iter()
        .map(Value::String)
        .collect::<Vec<_>>();
    evidence_lines.extend(
        array_field(natural_science_filter, "sourceEvidenceLines")
            .iter()
            .cloned(),
    );
    let mut filter = Map::new();
    filter.insert("filterId".to_string(), json!(filter_id));
    filter.insert(
        "filterKey".to_string(),
        json!("computer-engineering-math-science"),
    );
    filter.insert("label".to_string(), json!(label));
    filter.insert(
        "ownerIds".to_string(),
        json!(["uw-seattle-computer-engineering"]),
    );
    filter.insert("campusId".to_string(), json!("uw-seattle"));
    if let Some(source_url) = owner.get("sourceUrl") {
        filter.insert("officialSourceUrl".to_string(), source_url.clone());
        filter.insert("sourceUrl".to_string(), source_url.clone());
    }
    if let Some(source_role) = owner.get("sourceRole") {
        filter.insert("sourceRole".to_string(), source_role.clone());
    }
    filter.insert(
        "approvedUwCourseCodes".to_string(),
        json!(unique_sorted_strings(codes)),
    );
    if !generic_tags.is_empty() {
        filter.insert("genericCategoryTags".to_string(), json!(generic_tags));
    }
    filter.insert(
        "aliases".to_string(),
        json!(unique_in_order_strings(
            std::iter::once("computer-engineering-math-science".to_string()).chain(aliases)
        )),
    );
    filter.insert(
        "generatedFromOfficialSupportSource".to_string(),
        json!(true),
    );
    filter.insert("sourceEvidenceHeadings".to_string(), json!(headings));
    filter.insert(
        "sourceEvidenceLines".to_string(),
        json!(unique_in_order_values(evidence_lines)),
    );
    filter.insert("generatedAt".to_string(), generated_at.clone());
    let fingerprint = fingerprint_for_generated_filter(&Value::Object(filter.clone()));
    filter.insert("sourceFingerprint".to_string(), json!(fingerprint));
    Some(Value::Object(filter))
}

fn extract_computer_engineering_math_science_extra_codes(
    owner: &Value,
    snapshot_store: &RequirementSnapshotStore,
    course_parser: &CourseCodeParser,
) -> (Vec<String>, Vec<String>) {
    let lines = snapshot_lines_for_owner(owner, snapshot_store);
    let start_index = lines.iter().position(|line| {
        computer_engineering_math_science_heading_pattern().is_match(
            &super::requirement_source_audits::strip_source_line_noise(line),
        )
    });
    let Some(start_index) = start_index else {
        return (Vec::new(), Vec::new());
    };
    let evidence_lines = lines[start_index..lines.len().min(start_index + 8)]
        .iter()
        .map(|line| super::requirement_source_audits::strip_source_line_noise(line))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let mut codes = Vec::new();
    let mut parse_following_math_numbers = false;
    for line in &evidence_lines {
        codes.extend(course_parser.extract_from_line(line));
        if computer_engineering_math_fundamentals_pattern().is_match(line) {
            parse_following_math_numbers = true;
        } else if parse_following_math_numbers {
            let uppercase = line.to_ascii_uppercase();
            let amath_index = uppercase.match_indices("AMATH").find_map(|(index, _)| {
                let before = line[..index].chars().next_back();
                let after = line[index + "AMATH".len()..].chars().next();
                let is_word_character =
                    |character: char| character.is_ascii_alphanumeric() || character == '_';
                (!before.is_some_and(is_word_character) && !after.is_some_and(is_word_character))
                    .then_some(index)
            });
            let before_amath = amath_index
                .map(|index| &line[..index])
                .unwrap_or(line.as_str());
            codes.extend(
                super::requirement_source_audits::extract_approved_filter_codes_with_subject(
                    before_amath,
                    "MATH",
                    course_parser,
                ),
            );
            parse_following_math_numbers = false;
        }
    }
    (
        unique_sorted_strings(codes.into_iter().map(|code| normalize_course_code(&code))),
        evidence_lines,
    )
}

fn computer_engineering_math_science_heading_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)3\s+to\s+6\s+additional credits? of math/science")
            .expect("CE math-science heading")
    })
}

fn computer_engineering_math_fundamentals_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)MATH Fundamentals section above").expect("CE math fundamentals cue")
    })
}

fn fingerprint_for_generated_filter(filter: &Value) -> String {
    let mut fingerprint_input = Map::new();
    for key in [
        "filterKey",
        "ownerIds",
        "officialSourceUrl",
        "sourceUrl",
        "sourceRole",
        "approvedUwCourseCodes",
    ] {
        if let Some(value) = filter.get(key) {
            fingerprint_input.insert(key.to_string(), value.clone());
        }
    }
    for key in [
        "approvedUwCourseGroups",
        "petitionOnlyNotes",
        "sourceEvidenceLines",
    ] {
        let value = filter
            .get(key)
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or_else(|| json!([]));
        fingerprint_input.insert(key.to_string(), value);
    }
    fingerprint_json(&Value::Object(fingerprint_input))
}

fn compatibility_for_filter_key(key: &str) -> (String, String, Vec<String>, Vec<String>, bool) {
    match key {
        "computer-engineering-natural-science" => (
            "CE_APPROVED_NATURAL_SCIENCE".to_string(),
            "CE-approved Natural Science".to_string(),
            vec![
                "ce-approved-natural-science",
                "ce-approved-natural-sciences",
                "ce-natural-science",
                "ce-approved-nsc",
                "CE_APPROVED_NATURAL_SCIENCE",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            vec!["NSC", "NW"].into_iter().map(str::to_string).collect(),
            false,
        ),
        "computer-engineering-math-science" => (
            "CE_APPROVED_MATH_SCIENCE".to_string(),
            "CE-approved Math/Science".to_string(),
            vec![
                "ce-approved-math-science",
                "ce-math-science",
                "computer-engineering-approved-math-science",
                "CE_APPROVED_MATH_SCIENCE",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            vec!["NSC", "NW"].into_iter().map(str::to_string).collect(),
            false,
        ),
        "computer-science-approved-science" => (
            "CS_APPROVED_SCIENCE".to_string(),
            "CS-approved Science".to_string(),
            vec![
                "cs-approved-science",
                "cs-approved-natural-science",
                "computer-science-natural-science",
                "CS_APPROVED_SCIENCE",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            vec!["NSC", "NW"].into_iter().map(str::to_string).collect(),
            true,
        ),
        _ => {
            let tokens = key
                .split('-')
                .filter(|token| !token.is_empty())
                .collect::<Vec<_>>();
            let label = tokens
                .iter()
                .map(|token| {
                    let mut chars = token.chars();
                    chars
                        .next()
                        .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join(" ");
            let generic_tags = if key.split('-').any(|token| token == "science") {
                vec!["NSC".to_string(), "NW".to_string()]
            } else {
                Vec::new()
            };
            (
                key.to_ascii_uppercase().replace('-', "_"),
                label,
                Vec::new(),
                generic_tags,
                false,
            )
        }
    }
}

fn preferred_official_source_url(records: &[FilterRecord]) -> Option<Value> {
    records
        .iter()
        .find(|record| {
            record.owner.get("sourceRole").and_then(Value::as_str) == Some("approved-course-list")
        })
        .and_then(|record| record.owner.get("sourceUrl"))
        .filter(|value| !value.is_null())
        .cloned()
        .or_else(|| {
            records.iter().find_map(|record| {
                record
                    .support_list
                    .get("sourceUrl")
                    .filter(|value| truthy(value))
                    .cloned()
            })
        })
        .or_else(|| {
            records.first().and_then(|record| {
                record
                    .owner
                    .get("sourceUrl")
                    .filter(|value| !value.is_null())
                    .cloned()
            })
        })
}

fn snapshot_lines_for_owner(owner: &Value, store: &RequirementSnapshotStore) -> Vec<String> {
    if let Some(lines) = owner.get("snapshotLines").and_then(Value::as_array) {
        if !lines.is_empty() {
            return lines
                .iter()
                .map(js_string)
                .map(|line| super::requirement_shape::normalize_js_whitespace(&line))
                .filter(|line| !line.is_empty())
                .collect();
        }
    }
    let Some(snapshot_path) = owner
        .get("snapshotPath")
        .and_then(Value::as_str)
        .map(|path| Path::new(path))
    else {
        return Vec::new();
    };
    let source_url = owner.get("sourceUrl").and_then(Value::as_str);
    if let Some(snapshot) = store
        .read_snapshot_file(snapshot_path, source_url)
        .or_else(|| store.read_snapshot_file(snapshot_path, None))
    {
        return snapshot.snapshot_lines;
    }
    let Some(file_name) = snapshot_path.file_name() else {
        return Vec::new();
    };
    let local_snapshot_path = store.directory().join(file_name);
    store
        .read_snapshot_file(&local_snapshot_path, source_url)
        .or_else(|| store.read_snapshot_file(&local_snapshot_path, None))
        .map(|snapshot| snapshot.snapshot_lines)
        .unwrap_or_default()
}

fn read_all_snapshots(
    store: &RequirementSnapshotStore,
) -> Vec<super::requirement_snapshots::RequirementSourceSnapshot> {
    let Ok(entries) = std::fs::read_dir(store.directory()) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension().and_then(|extension| extension.to_str()) == Some("txt"))
                .then(|| store.read_snapshot_file(&path, None))
                .flatten()
        })
        .collect()
}

fn fingerprint_json(value: &Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_else(|_| b"null".to_vec());
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn normalize_filter_key(value: &str) -> String {
    let mut normalized = String::new();
    let mut hyphen = false;
    for character in value.trim().to_ascii_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            normalized.push(character);
            hyphen = false;
        } else if !normalized.is_empty() && !hyphen {
            normalized.push('-');
            hyphen = true;
        }
    }
    while normalized.ends_with('-') {
        normalized.pop();
    }
    normalized
}

fn unique_sorted_strings(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut values = values
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn unique_in_order_strings(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    values
        .into_iter()
        .filter(|value| !value.is_empty() && seen.insert(value.clone()))
        .collect()
}

fn unique_sorted_values(values: Vec<Value>) -> Value {
    let mut seen = std::collections::HashSet::new();
    let mut values = values
        .into_iter()
        .filter(|value| truthy(value))
        .filter(|value| seen.insert(js_string(value)))
        .collect::<Vec<_>>();
    values.sort_by_key(js_string);
    json!(values)
}

fn unique_in_order_values(values: Vec<Value>) -> Vec<Value> {
    let mut seen = std::collections::HashSet::new();
    values
        .into_iter()
        .filter(|value| truthy(value))
        .filter(|value| seen.insert(js_string(value)))
        .collect()
}

fn array_field<'a>(owner: &'a Value, key: &str) -> &'a [Value] {
    owner
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
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
