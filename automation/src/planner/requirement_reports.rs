use chrono::{SecondsFormat, Utc};
use serde_json::{Value, json};

pub fn build_uw_mse_course_extraction_audit(owners: &[Value]) -> Value {
    owners
        .iter()
        .find(|owner| {
            owner.get("planId").and_then(Value::as_str)
                == Some("uw-seattle-materials-science-engineering")
                && !js_truthy(owner.get("pathwayId"))
        })
        .map(|owner| {
            super::requirement_source_quality::build_uw_mse_course_extraction_audit_for_owner(owner)
        })
        .unwrap_or(Value::Null)
}

pub fn build_parser_recovery_markdown_report(report: &Value) -> String {
    let mut lines = vec![
        "# Transfer Planner Parser Recovery Report".to_string(),
        String::new(),
        format!("Generated: {}", js_interpolate(report.get("generatedAt"))),
        String::new(),
        format!(
            "- Owners triggering parser recovery: {}",
            js_interpolate(report.get("triggeredOwnerCount"))
        ),
        format!(
            "- Owners recovered: {}",
            js_interpolate(report.get("successfulOwnerCount"))
        ),
        format!(
            "- Owners unrecovered: {}",
            js_interpolate(report.get("unrecoveredOwnerCount"))
        ),
        format!(
            "- Owners with recovered schedulable sources: {}",
            js_interpolate(report.get("recoveredScheduledSourceOwnerCount"))
        ),
        format!(
            "- Owners with recovered support sources: {}",
            js_interpolate(report.get("recoveredSupportSourceOwnerCount"))
        ),
        String::new(),
        "## Trigger Codes".to_string(),
        String::new(),
    ];
    lines.extend(
        sorted_object_entries(report.get("countsByTriggerCode"))
            .into_iter()
            .map(|(code, count)| format!("- {code}: {}", js_interpolate(Some(count)))),
    );
    lines.extend([
        String::new(),
        "## Attempted Strategies".to_string(),
        String::new(),
    ]);
    lines.extend(
        sorted_object_entries(report.get("countsByAttemptedStrategy"))
            .into_iter()
            .map(|(strategy, count)| format!("- {strategy}: {}", js_interpolate(Some(count)))),
    );
    lines.extend([
        String::new(),
        "## Unrecovered Blockers".to_string(),
        String::new(),
    ]);
    lines.extend(
        sorted_object_entries(report.get("countsByBlockerType"))
            .into_iter()
            .map(|(blocker, count)| format!("- {blocker}: {}", js_interpolate(Some(count)))),
    );
    lines.push(String::new());

    let owners = report
        .get("owners")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let recovered = owners
        .iter()
        .filter(|owner| js_truthy(owner.get("succeeded")))
        .take(80)
        .collect::<Vec<_>>();
    if !recovered.is_empty() {
        lines.extend(["## Recovered Owners".to_string(), String::new()]);
        for owner in recovered {
            lines.push(format!("### {}", js_interpolate(owner.get("ownerTitle"))));
            lines.push(String::new());
            lines.push(format!(
                "- Owner id: {}",
                js_interpolate(owner.get("ownerId"))
            ));
            lines.push(format!(
                "- Original source: {}",
                js_interpolate(owner.get("originalSourceUrl"))
            ));
            lines.push(format!(
                "- Trigger: {}",
                array_join(owner.get("triggerCodes"), ", ", "n/a")
            ));
            lines.push(format!(
                "- Attempted strategies: {}",
                array_join(owner.get("attemptedStrategies"), ", ", "n/a")
            ));
            lines.push(format!(
                "- Before/after parsed UW course count: {} -> {}",
                nested_or_default(owner, &["before", "parsedUwCourseCodeCount"], "0"),
                nested_or_default(owner, &["after", "parsedUwCourseCodeCount"], "0")
            ));
            lines.push(format!(
                "- Before/after warnings: {} -> {}",
                array_join(
                    nested(owner, &["before", "qualityWarningCodes"]),
                    ", ",
                    "none"
                ),
                array_join(
                    nested(owner, &["after", "qualityWarningCodes"]),
                    ", ",
                    "none"
                )
            ));
            for source in owner
                .get("recoveredSources")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                lines.push(format!(
                    "- Recovered source: {}; {}; parsed={}; fingerprint={}",
                    js_interpolate(source.get("strategy")),
                    js_interpolate(source.get("sourceUrl")),
                    js_interpolate(source.get("parsedUwCourseCodeCount")),
                    js_interpolate(source.get("sourceEvidenceFingerprint"))
                ));
            }
            for source in owner
                .get("supportSources")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                lines.push(format!(
                    "- Support source: {}; {}; accepted={}; fingerprint={}",
                    js_interpolate(source.get("strategy")),
                    js_interpolate(source.get("sourceUrl")),
                    js_interpolate(source.get("acceptedUwCourseCodeCount")),
                    js_interpolate(source.get("sourceEvidenceFingerprint"))
                ));
            }
            lines.push(String::new());
        }
    }

    let unrecovered = owners
        .iter()
        .filter(|owner| !js_truthy(owner.get("succeeded")))
        .take(120)
        .collect::<Vec<_>>();
    if !unrecovered.is_empty() {
        lines.extend(["## Unrecovered Owners".to_string(), String::new()]);
        for owner in unrecovered {
            lines.push(format!("### {}", js_interpolate(owner.get("ownerTitle"))));
            lines.push(String::new());
            lines.push(format!(
                "- Owner id: {}",
                js_interpolate(owner.get("ownerId"))
            ));
            lines.push(format!(
                "- Original source: {}",
                js_interpolate(owner.get("originalSourceUrl"))
            ));
            lines.push(format!(
                "- Trigger: {}",
                array_join(owner.get("triggerCodes"), ", ", "n/a")
            ));
            lines.push(format!(
                "- Blocker: {}",
                nullish_or(owner.get("blockerType"), "unknown")
            ));
            lines.push(format!(
                "- Attempted strategies: {}",
                array_join(owner.get("attemptedStrategies"), ", ", "none")
            ));
            lines.push(format!(
                "- Before/after parsed UW course count: {} -> {}",
                nested_or_default(owner, &["before", "parsedUwCourseCodeCount"], "0"),
                nested_or_default(owner, &["after", "parsedUwCourseCodeCount"], "0")
            ));
            lines.push(String::new());
        }
    }

    format!("{}\n", lines.join("\n"))
}

fn sorted_object_entries<'a>(value: Option<&'a Value>) -> Vec<(String, &'a Value)> {
    let Some(object) = value.and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut entries = object
        .iter()
        .map(|(key, value)| (key.clone(), value))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| report_locale_compare(&left.0, &right.0));
    entries
}

fn report_locale_compare(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| right.cmp(left))
}

fn nested<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
}

fn nested_or_default(value: &Value, path: &[&str], default: &str) -> String {
    let value = nested(value, path);
    if value.is_none_or(Value::is_null) {
        default.to_string()
    } else {
        js_interpolate(value)
    }
}

fn nullish_or(value: Option<&Value>, default: &str) -> String {
    if value.is_none_or(Value::is_null) {
        default.to_string()
    } else {
        js_interpolate(value)
    }
}

fn array_join(value: Option<&Value>, separator: &str, empty_fallback: &str) -> String {
    let Some(items) = value.and_then(Value::as_array) else {
        return empty_fallback.to_string();
    };
    let joined = items
        .iter()
        .map(|item| {
            if item.is_null() {
                String::new()
            } else {
                js_interpolate(Some(item))
            }
        })
        .collect::<Vec<_>>()
        .join(separator);
    if joined.is_empty() {
        empty_fallback.to_string()
    } else {
        joined
    }
}

pub fn build_parser_recovery_report(owners: &[Value]) -> Value {
    let recovered_owners = owners
        .iter()
        .filter_map(|owner| {
            let recovery = owner.get("parserRecovery")?;
            (js_truthy(recovery.get("triggered")))
                .then(|| build_parser_recovery_report_owner(owner, recovery))
        })
        .collect::<Vec<_>>();
    let successful_owners = recovered_owners
        .iter()
        .filter(|owner| owner.get("succeeded").and_then(Value::as_bool) == Some(true))
        .collect::<Vec<_>>();
    let unrecovered_owners = recovered_owners
        .iter()
        .filter(|owner| owner.get("succeeded").and_then(Value::as_bool) != Some(true))
        .collect::<Vec<_>>();
    let recovered_scheduled_count = recovered_owners
        .iter()
        .filter(|owner| nonempty_array(owner.get("recoveredSources")))
        .count();
    let recovered_support_count = recovered_owners
        .iter()
        .filter(|owner| nonempty_array(owner.get("supportSources")))
        .count();
    let trigger_codes = recovered_owners
        .iter()
        .flat_map(|owner| array_strings(owner.get("triggerCodes")))
        .collect::<Vec<_>>();
    let attempted_strategies = recovered_owners
        .iter()
        .flat_map(|owner| array_strings(owner.get("attemptedStrategies")))
        .collect::<Vec<_>>();
    let blocker_types = unrecovered_owners
        .iter()
        .map(|owner| {
            owner
                .get("blockerType")
                .filter(|value| !value.is_null())
                .map(|value| js_interpolate(Some(value)))
                .unwrap_or_else(|| "unknown".to_string())
        })
        .collect::<Vec<_>>();
    json!({
        "generatedAt": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        "triggeredOwnerCount": recovered_owners.len(),
        "successfulOwnerCount": successful_owners.len(),
        "unrecoveredOwnerCount": unrecovered_owners.len(),
        "recoveredScheduledSourceOwnerCount": recovered_scheduled_count,
        "recoveredSupportSourceOwnerCount": recovered_support_count,
        "countsByTriggerCode": count_strings(trigger_codes),
        "countsByAttemptedStrategy": count_strings(attempted_strategies),
        "countsByBlockerType": count_strings(blocker_types),
        "owners": recovered_owners,
    })
}

fn build_parser_recovery_report_owner(owner: &Value, recovery: &Value) -> Value {
    let mut report = serde_json::Map::new();
    for (key, value) in [
        ("ownerId", owner.get("ownerId")),
        ("ownerTitle", owner.get("ownerTitle")),
        ("planId", owner.get("planId")),
    ] {
        if let Some(value) = value {
            report.insert(key.to_string(), value.clone());
        }
    }
    report.insert(
        "pathwayId".to_string(),
        owner
            .get("pathwayId")
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or(Value::Null),
    );
    for (key, value) in [
        ("campusId", owner.get("campusId")),
        ("originalSourceUrl", recovery.get("originalSourceUrl")),
    ] {
        if let Some(value) = value {
            report.insert(key.to_string(), value.clone());
        }
    }
    for key in [
        "triggerCodes",
        "attemptedStrategies",
        "recoveredSources",
        "supportSources",
    ] {
        report.insert(
            key.to_string(),
            recovery
                .get(key)
                .filter(|value| !value.is_null())
                .cloned()
                .unwrap_or_else(|| json!([])),
        );
    }
    for key in ["before", "after"] {
        if let Some(value) = recovery.get(key) {
            report.insert(key.to_string(), value.clone());
        }
    }
    report.insert(
        "blockerType".to_string(),
        recovery
            .get("blockerType")
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or(Value::Null),
    );
    report.insert(
        "succeeded".to_string(),
        json!(recovery.get("succeeded").and_then(Value::as_bool) == Some(true)),
    );
    Value::Object(report)
}

fn nonempty_array(value: Option<&Value>) -> bool {
    super::requirement_source_quality::js_length_is_positive(value)
}

fn array_strings(value: Option<&Value>) -> Vec<String> {
    match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(values)) => values
            .iter()
            .map(|item| js_interpolate(Some(item)))
            .collect(),
        Some(value) => vec![js_interpolate(Some(value))],
    }
}

fn count_strings(values: Vec<String>) -> Value {
    let mut counts = serde_json::Map::new();
    for value in values {
        let next_count = counts.get(&value).and_then(Value::as_u64).unwrap_or(0) + 1;
        counts.insert(value, json!(next_count));
    }
    Value::Object(counts)
}

fn js_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_f64().is_some_and(|value| value != 0.0),
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

fn js_interpolate(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(Value::String(value)) => value.clone(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Number(value)) => super::requirement_shape::js_number_to_string(value),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                if value.is_null() {
                    String::new()
                } else {
                    js_interpolate(Some(value))
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".to_string(),
    }
}
