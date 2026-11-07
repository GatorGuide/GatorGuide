use serde_json::Value;

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
        format!("- Owners recovered: {}", js_interpolate(report.get("successfulOwnerCount"))),
        format!("- Owners unrecovered: {}", js_interpolate(report.get("unrecoveredOwnerCount"))),
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
    lines.extend(sorted_object_entries(report.get("countsByTriggerCode")).into_iter().map(
        |(code, count)| format!("- {code}: {}", js_interpolate(Some(count))),
    ));
    lines.extend([
        String::new(),
        "## Attempted Strategies".to_string(),
        String::new(),
    ]);
    lines.extend(sorted_object_entries(report.get("countsByAttemptedStrategy")).into_iter().map(
        |(strategy, count)| format!("- {strategy}: {}", js_interpolate(Some(count))),
    ));
    lines.extend([
        String::new(),
        "## Unrecovered Blockers".to_string(),
        String::new(),
    ]);
    lines.extend(sorted_object_entries(report.get("countsByBlockerType")).into_iter().map(
        |(blocker, count)| format!("- {blocker}: {}", js_interpolate(Some(count))),
    ));
    lines.push(String::new());

    let owners = report
        .get("owners")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let recovered = owners
        .iter()
        .filter(|owner| owner.get("succeeded").and_then(Value::as_bool) == Some(true))
        .take(80)
        .collect::<Vec<_>>();
    if !recovered.is_empty() {
        lines.extend(["## Recovered Owners".to_string(), String::new()]);
        for owner in recovered {
            lines.push(format!("### {}", js_interpolate(owner.get("ownerTitle"))));
            lines.push(String::new());
            lines.push(format!("- Owner id: {}", js_interpolate(owner.get("ownerId"))));
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
        .filter(|owner| owner.get("succeeded").and_then(Value::as_bool) != Some(true))
        .take(120)
        .collect::<Vec<_>>();
    if !unrecovered.is_empty() {
        lines.extend(["## Unrecovered Owners".to_string(), String::new()]);
        for owner in unrecovered {
            lines.push(format!("### {}", js_interpolate(owner.get("ownerTitle"))));
            lines.push(String::new());
            lines.push(format!("- Owner id: {}", js_interpolate(owner.get("ownerId"))));
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
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

fn nested<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |current, key| current.get(*key))
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

fn js_interpolate(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(Value::String(value)) => value.clone(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| if value.is_null() { String::new() } else { js_interpolate(Some(value)) })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".to_string(),
    }
}
