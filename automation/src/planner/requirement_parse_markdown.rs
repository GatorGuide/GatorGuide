use serde_json::Value;

const DEFAULT_CAMPUS_ORDER: [&str; 3] = ["uw-seattle", "uw-bothell", "uw-tacoma"];

pub fn build_requirement_source_parse_markdown_report(report: &Value, options: &Value) -> String {
    format!(
        "{}\n",
        build_requirement_source_parse_markdown_lines(report, options).join("\n")
    )
}

pub fn build_requirement_source_parse_markdown_lines(
    report: &Value,
    options: &Value,
) -> Vec<String> {
    let campus_order = options
        .get("campusOrder")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| js_string(Some(item)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            DEFAULT_CAMPUS_ORDER
                .iter()
                .map(|campus| campus.to_string())
                .collect()
        });
    let recovery_markdown_path = nullish_string(options.get("recoveryMarkdownPath"), "n/a");
    let mut lines = vec![
        "# Transfer Planner Requirement Source Parse Report".to_string(),
        String::new(),
        format!("Generated: {}", js_string(report.get("generatedAt"))),
        String::new(),
        format!(
            "- Primary degree sources parsed: {}",
            js_string(report.get("totalOwners"))
        ),
        format!(
            "- Parsed successfully: {}",
            js_string(report.get("okCount"))
        ),
        format!("- Parse failures: {}", js_string(report.get("failedCount"))),
        format!(
            "- Parsed requirement source adapter blocks: {}",
            js_string(report.get("parsedRequirementSourceBlockCount"))
        ),
        format!(
            "- Parsed requirement atom candidates: {}",
            js_string(report.get("parsedRequirementAtomCandidateCount"))
        ),
        format!(
            "- Parsed degree-map block candidates: {}",
            js_string(report.get("parsedDegreeMapBlockCandidateCount"))
        ),
        format!(
            "- Parsed structured requirement course entries: {}",
            js_string(report.get("parsedRequirementCourseCount"))
        ),
        format!(
            "- Parsed from cached snapshots after live-source failures: {}",
            js_string(report.get("snapshotFallbackCount"))
        ),
        format!(
            "- Parsed from alternate official source URLs: {}",
            nullish_string(
                nested(
                    report,
                    &["countsByResolutionStrategy", "alternate-official-source"]
                ),
                "0"
            )
        ),
        format!(
            "- Owners with parsed UW course codes: {}",
            js_string(report.get("withParsedCourseCodesCount"))
        ),
        format!(
            "- Owners with source-only UW course codes not currently in structured degree-map blocks: {}",
            js_string(report.get("withSourceOnlyCourseCodesCount"))
        ),
        format!(
            "- Owners with no parsed UW course codes: {}",
            js_string(report.get("withNoParsedCourseCodesCount"))
        ),
        format!(
            "- Owners with parser-quality warnings: {}",
            js_string(report.get("ownersWithQualityWarningsCount"))
        ),
        format!(
            "- Owners with parser-quality notes: {}",
            js_string(report.get("ownersWithQualityNotesCount"))
        ),
        format!(
            "- Owners triggering parser auto-recovery: {}",
            nullish_string(
                nested(report, &["parserRecoveryReport", "triggeredOwnerCount"]),
                "0"
            )
        ),
        format!(
            "- Owners recovered by parser auto-recovery: {}",
            nullish_string(
                nested(report, &["parserRecoveryReport", "successfulOwnerCount"]),
                "0"
            )
        ),
        String::new(),
        "## Parser Adapters".to_string(),
        String::new(),
    ];
    append_count_section(&mut lines, report.get("countsByAdapterId"));
    lines.extend([
        String::new(),
        "## Resolution Strategies".to_string(),
        String::new(),
    ]);
    append_count_section(&mut lines, report.get("countsByResolutionStrategy"));
    lines.extend([String::new(), "## Source Roles".to_string(), String::new()]);
    append_count_section(&mut lines, report.get("countsBySourceRole"));
    lines.extend([
        String::new(),
        "## Source Role Statuses".to_string(),
        String::new(),
    ]);
    append_count_section(&mut lines, report.get("countsBySourceRoleStatus"));
    lines.extend([
        format!(
            "- can-create-schedulable-rows: {}",
            nullish_string(report.get("canCreateSchedulableRowCount"), "0")
        ),
        format!(
            "- can-create-required-rows: {}",
            nullish_string(report.get("canCreateRequiredRowCount"), "0")
        ),
        format!(
            "- can-create-option-groups: {}",
            nullish_string(report.get("canCreateOptionGroupCount"), "0")
        ),
        format!(
            "- can-create-approved-filters: {}",
            nullish_string(report.get("canCreateApprovedFilterCount"), "0")
        ),
        format!(
            "- can-create-elective-lists: {}",
            nullish_string(report.get("canCreateElectiveListCount"), "0")
        ),
        format!(
            "- support-only sources: {}",
            nullish_string(report.get("supportOnlySourceCount"), "0")
        ),
        format!(
            "- non-schedulable sources: {}",
            nullish_string(report.get("nonSchedulableSourceCount"), "0")
        ),
        String::new(),
        "## Parser Quality Signals".to_string(),
        String::new(),
    ]);
    append_count_section(&mut lines, report.get("countsByQualitySignalCode"));
    lines.extend([
        String::new(),
        "## Parser Auto-Recovery".to_string(),
        String::new(),
        format!(
            "- Triggered owners: {}",
            nullish_string(
                nested(report, &["parserRecoveryReport", "triggeredOwnerCount"]),
                "0"
            )
        ),
        format!(
            "- Successful recoveries: {}",
            nullish_string(
                nested(report, &["parserRecoveryReport", "successfulOwnerCount"]),
                "0"
            )
        ),
        format!(
            "- Unrecovered owners: {}",
            nullish_string(
                nested(report, &["parserRecoveryReport", "unrecoveredOwnerCount"]),
                "0"
            )
        ),
        format!(
            "- Recovered schedulable source owners: {}",
            nullish_string(
                nested(
                    report,
                    &["parserRecoveryReport", "recoveredScheduledSourceOwnerCount"]
                ),
                "0"
            )
        ),
        format!(
            "- Recovered support source owners: {}",
            nullish_string(
                nested(
                    report,
                    &["parserRecoveryReport", "recoveredSupportSourceOwnerCount"]
                ),
                "0"
            )
        ),
        format!("- Detailed report: {recovery_markdown_path}"),
        String::new(),
    ]);

    append_mse_audit(&mut lines, report.get("uwMseCourseExtractionAudit"));
    append_string_audit_section(
        &mut lines,
        "## Source Section Audit",
        report.get("sourceSectionAuditLines"),
        None,
    );
    append_string_audit_section(
        &mut lines,
        "## Parser Prerequisite Filter Audit",
        report.get("sourceSectionFilterAuditLines"),
        Some("additional parser prerequisite filter audit rows omitted from markdown"),
    );
    append_string_audit_section(
        &mut lines,
        "## Parser Sequence-Choice Audit",
        report.get("parserSequenceChoiceAuditLines"),
        Some("additional parser sequence-choice audit rows omitted from markdown"),
    );
    append_string_audit_section(
        &mut lines,
        "## Source Scope Audit",
        report.get("sourceScopeAuditLines"),
        Some("additional source-scope audit rows omitted from markdown"),
    );

    let owners = report
        .get("owners")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for campus_id in campus_order {
        let campus_owners = owners
            .iter()
            .filter(|owner| {
                owner.get("campusId").and_then(Value::as_str) == Some(campus_id.as_str())
            })
            .collect::<Vec<_>>();
        if campus_owners.is_empty() {
            continue;
        }
        lines.extend([format!("## {campus_id}"), String::new()]);

        let quality_owners = campus_owners
            .iter()
            .copied()
            .filter(|owner| {
                array_field(owner, "qualitySignals")
                    .iter()
                    .any(|signal| signal.get("severity").and_then(Value::as_str) == Some("warning"))
            })
            .collect::<Vec<_>>();
        if !quality_owners.is_empty() {
            lines.extend(["### Parser-quality warnings".to_string(), String::new()]);
            for owner in quality_owners.into_iter().take(50) {
                lines.extend([
                    format!("#### {}", js_string(owner.get("ownerTitle"))),
                    String::new(),
                    format!("- Source: {}", js_string(owner.get("sourceUrl"))),
                    format!(
                        "- Parse confidence: {}",
                        js_string(owner.get("parseConfidence"))
                    ),
                ]);
                let warning_text = array_field(owner, "qualitySignals")
                    .iter()
                    .filter(|signal| {
                        signal.get("severity").and_then(Value::as_str) == Some("warning")
                    })
                    .map(|signal| {
                        let code = js_string(signal.get("code"));
                        match signal
                            .get("details")
                            .filter(|details| js_truthy(Some(*details)))
                        {
                            Some(details) => format!("{code} ({})", js_string(Some(details))),
                            None => code,
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" | ");
                lines.push(format!("- Quality warnings: {warning_text}"));
                if owner.get("primarySourceUrl") != owner.get("sourceUrl") {
                    lines.push(format!(
                        "- Primary source: {}",
                        js_string(owner.get("primarySourceUrl"))
                    ));
                }
                lines.push(String::new());
            }
        }

        let drift_owners = campus_owners
            .iter()
            .copied()
            .filter(|owner| !array_field(owner, "sourceOnlyUwCourseCodes").is_empty())
            .collect::<Vec<_>>();
        if !drift_owners.is_empty() {
            lines.extend([
                "### Possible source-vs-structured drift".to_string(),
                String::new(),
            ]);
            for owner in drift_owners.into_iter().take(50) {
                lines.extend([
                    format!("#### {}", js_string(owner.get("ownerTitle"))),
                    String::new(),
                    format!("- Source: {}", js_string(owner.get("sourceUrl"))),
                ]);
                if owner.get("primarySourceUrl") != owner.get("sourceUrl") {
                    lines.push(format!(
                        "- Primary source: {}",
                        js_string(owner.get("primarySourceUrl"))
                    ));
                }
                lines.extend([
                    format!("- Parser type: {}", js_string(owner.get("parserType"))),
                    format!(
                        "- Source role: {}",
                        nullish_string(owner.get("sourceRole"), "ignored")
                    ),
                    format!("- Source role status: {}", role_status(owner)),
                    format!(
                        "- Can create schedulable rows: {}",
                        if owner
                            .get("canCreateSchedulableRows")
                            .and_then(Value::as_bool)
                            == Some(false)
                        {
                            "no"
                        } else {
                            "yes"
                        }
                    ),
                    format!("- Parser adapter: {}", js_string(owner.get("adapterId"))),
                    format!(
                        "- Resolution strategy: {}",
                        js_string(owner.get("resolutionStrategy"))
                    ),
                    format!(
                        "- Parse confidence: {}",
                        js_string(owner.get("parseConfidence"))
                    ),
                ]);
                if js_truthy(owner.get("usedSnapshotFallback")) {
                    lines.push(format!(
                        "- Snapshot fallback: {}",
                        nullish_string(
                            owner.get("snapshotFallbackReason"),
                            "used cached source snapshot"
                        )
                    ));
                }
                lines.push(format!(
                    "- Source-only UW course codes: {}",
                    array_join(array_field(owner, "sourceOnlyUwCourseCodes"), ", ")
                ));
                if !array_field(owner, "structuredOnlyUwCourseCodes").is_empty() {
                    lines.push(format!(
                        "- Structured-only UW course codes not seen in the parsed source: {}",
                        array_join(array_field(owner, "structuredOnlyUwCourseCodes"), ", ")
                    ));
                }
                let cues = array_field(owner, "requirementCueLines");
                if !cues.is_empty() {
                    lines.push(format!(
                        "- Requirement cues: {}",
                        array_join(&cues[..cues.len().min(3)], " | ")
                    ));
                }
                if let Some(line) = nested(owner, &["sourceSectionAudit", "line"])
                    .filter(|line| js_truthy(Some(*line)))
                {
                    lines.push(format!("- {}", js_string(Some(line))));
                }
                lines.push(format!(
                    "- Snapshot: {}",
                    nullish_string(owner.get("snapshotPath"), "n/a")
                ));
                lines.push(String::new());
            }
        }

        let no_course_owners = campus_owners
            .iter()
            .copied()
            .filter(|owner| {
                js_truthy(owner.get("ok")) && array_field(owner, "parsedUwCourseCodes").is_empty()
            })
            .collect::<Vec<_>>();
        if !no_course_owners.is_empty() {
            lines.extend([
                "### Parsed but no UW course codes found".to_string(),
                String::new(),
            ]);
            for owner in no_course_owners.into_iter().take(50) {
                lines.extend([
                    format!("- {}", js_string(owner.get("ownerTitle"))),
                    format!("  - Source: {}", js_string(owner.get("sourceUrl"))),
                    format!(
                        "  - Source role: {}",
                        nullish_string(owner.get("sourceRole"), "ignored")
                    ),
                    format!("  - Source role status: {}", role_status(owner)),
                    format!(
                        "  - Can create schedulable rows: {}",
                        if owner
                            .get("canCreateSchedulableRows")
                            .and_then(Value::as_bool)
                            == Some(false)
                        {
                            "no"
                        } else {
                            "yes"
                        }
                    ),
                    format!("  - Parser type: {}", js_string(owner.get("parserType"))),
                ]);
                if let Some(line) = nested(owner, &["sourceSectionAudit", "line"])
                    .filter(|line| js_truthy(Some(*line)))
                {
                    lines.push(format!("  - {}", js_string(Some(line))));
                }
                lines.push(format!(
                    "  - Requirement cues found: {}",
                    array_field(owner, "requirementCueLines").len()
                ));
            }
            lines.push(String::new());
        }

        let failed_owners = campus_owners
            .iter()
            .copied()
            .filter(|owner| !js_truthy(owner.get("ok")))
            .collect::<Vec<_>>();
        if !failed_owners.is_empty() {
            lines.extend(["### Parse failures".to_string(), String::new()]);
            for owner in failed_owners.into_iter().take(50) {
                lines.extend([
                    format!("- {}", js_string(owner.get("ownerTitle"))),
                    format!("  - Source: {}", js_string(owner.get("sourceUrl"))),
                    format!(
                        "  - Source role: {}",
                        nullish_string(owner.get("sourceRole"), "ignored")
                    ),
                    format!("  - Source role status: {}", role_status(owner)),
                    format!(
                        "  - Error: {}",
                        nullish_string(owner.get("error"), "unknown error")
                    ),
                ]);
            }
            lines.push(String::new());
        }
    }
    lines
}

fn append_count_section(lines: &mut Vec<String>, counts: Option<&Value>) {
    let Some(counts) = counts.and_then(Value::as_object) else {
        return;
    };
    let mut entries = counts.iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| locale_compare(left.0, right.0));
    lines.extend(
        entries
            .into_iter()
            .map(|(key, value)| format!("- {key}: {}", js_string(Some(value)))),
    );
}

fn append_mse_audit(lines: &mut Vec<String>, audit: Option<&Value>) {
    let Some(audit) = audit.filter(|audit| js_truthy(Some(*audit))) else {
        return;
    };
    lines.extend([
        "## UW MSE Course Extraction Audit".to_string(),
        String::new(),
        format!(
            "- Total parsed official entries: {}",
            js_string(audit.get("totalParsedOfficialEntries"))
        ),
    ]);
    let required = nested(audit, &["requiredVsOptionCounts", "required"])
        .map(|value| js_string(Some(value)))
        .unwrap_or_else(|| "undefined".to_string());
    let option = nested(audit, &["requiredVsOptionCounts", "option"])
        .map(|value| js_string(Some(value)))
        .unwrap_or_else(|| "undefined".to_string());
    let alias = nested(audit, &["requiredVsOptionCounts", "alias"])
        .map(|value| js_string(Some(value)))
        .unwrap_or_else(|| "undefined".to_string());
    let note = nested(audit, &["requiredVsOptionCounts", "noteOnly"])
        .map(|value| js_string(Some(value)))
        .unwrap_or_else(|| "undefined".to_string());
    lines.push(format!(
        "- Required / option / alias / note-only counts: {required} / {option} / {alias} / {note}"
    ));
    for (label, key) in [
        ("Missing expected courses", "missingExpectedCourses"),
        (
            "Duplicate normalized course codes",
            "duplicateNormalizedCourseCodes",
        ),
        ("Unclassified course codes", "unclassifiedCourseCodes"),
        (
            "Courses missing group/type/source metadata",
            "coursesMissingRequiredMetadata",
        ),
    ] {
        let values = array_field(audit, key);
        lines.push(format!(
            "- {label}: {}",
            if values.is_empty() {
                "none".to_string()
            } else {
                array_join(values, ", ")
            }
        ));
    }
    lines.push("- Grouped counts by category:".to_string());
    if let Some(counts) = audit
        .get("groupedCountsByCategory")
        .and_then(Value::as_object)
    {
        let mut entries = counts.iter().collect::<Vec<_>>();
        entries.sort_by(|left, right| locale_compare(left.0, right.0));
        for (category, count) in entries {
            lines.push(format!("  - {category}: {}", js_string(Some(count))));
        }
    }
    let notes = array_field(audit, "noteOrRestrictionEntries");
    if !notes.is_empty() {
        lines.push("- Courses with notes or restrictions:".to_string());
        for entry in notes.iter().take(25) {
            lines.push(format!(
                "  - {}: {}",
                js_string(entry.get("normalizedCourseCode")),
                array_join(array_field(entry, "notes"), " | ")
            ));
        }
    }
    lines.push(String::new());
}

fn append_string_audit_section(
    lines: &mut Vec<String>,
    title: &str,
    values: Option<&Value>,
    omission_label: Option<&str>,
) {
    let values = values
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if values.is_empty() {
        return;
    }
    lines.extend([title.to_string(), String::new()]);
    let limit = if omission_label.is_some() {
        400
    } else {
        values.len()
    };
    lines.extend(
        values
            .iter()
            .take(limit)
            .map(|value| format!("- {}", js_string(Some(value)))),
    );
    if let Some(label) = omission_label.filter(|_| values.len() > limit) {
        lines.push(format!("- ... {} {label}.", values.len() - limit));
    }
    lines.push(String::new());
}

fn locale_compare(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| right.cmp(left))
}

fn role_status(owner: &Value) -> String {
    if let Some(status) = owner
        .get("sourceRoleStatus")
        .filter(|value| !value.is_null())
    {
        return js_string(Some(status));
    }
    let role = owner
        .get("sourceRole")
        .filter(|value| !value.is_null())
        .map(|value| js_string(Some(value)))
        .unwrap_or_else(|| "ignored".to_string());
    super::requirement_scope::get_requirement_source_role_status(Some(&role))
        .as_str()
        .to_string()
}

fn nested<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
}

fn array_field<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn array_join(values: &[Value], separator: &str) -> String {
    values
        .iter()
        .map(|value| {
            if value.is_null() {
                String::new()
            } else {
                js_string(Some(value))
            }
        })
        .collect::<Vec<_>>()
        .join(separator)
}

fn nullish_string(value: Option<&Value>, fallback: &str) -> String {
    match value {
        None | Some(Value::Null) => fallback.to_string(),
        Some(value) => js_string(Some(value)),
    }
}

fn js_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_f64().is_some_and(|number| number != 0.0),
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Array(_)) | Some(Value::Object(_)) => true,
    }
}

fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Number(value)) => super::requirement_shape::js_number_to_string(value),
        Some(Value::String(value)) => value.clone(),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                if value.is_null() {
                    String::new()
                } else {
                    js_string(Some(value))
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".to_string(),
    }
}
