use serde::Deserialize;
use serde_json::{Map, Value, json};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EquivalencyGuideReportInput {
    pub generated_at: String,
    pub source_url: String,
    pub status: u16,
    pub final_url: String,
    pub content_type: Value,
    pub snapshot_path: String,
    pub snapshot_sha256: String,
    pub rows: Vec<Value>,
    pub official_rules: Vec<Value>,
    pub derived_rules: Vec<Value>,
}

pub fn build_equivalency_guide_report(input: &EquivalencyGuideReportInput) -> Value {
    let mut all_rules = input
        .official_rules
        .iter()
        .chain(input.derived_rules.iter())
        .cloned()
        .collect::<Vec<_>>();
    all_rules.sort_by(|left, right| string_field(left, "id").cmp(&string_field(right, "id")));

    let department_count = unique_row_departments(&input.rows);
    let rules_with_source_course_sets = all_rules
        .iter()
        .filter(|rule| {
            rule.get("sourceCourseSets")
                .and_then(Value::as_array)
                .and_then(|sets| sets.first())
                .and_then(Value::as_array)
                .is_some_and(|set| !set.is_empty())
        })
        .count();
    let rules_with_target_course_codes = all_rules
        .iter()
        .filter(|rule| {
            rule.get("targetCourseCodes")
                .and_then(Value::as_array)
                .is_some_and(|codes| !codes.is_empty())
        })
        .count();

    let counts_by_type = counts_by(&all_rules, |rule| property_key(rule.get("type")));
    let counts_by_status = counts_by(&all_rules, |rule| {
        nullish_property_key(rule.get("ruleStatus"), "unknown")
    });
    let counts_by_acceptance_category = counts_by(&all_rules, |rule| {
        property_key(rule.get("acceptanceCategory"))
    });
    let counts_by_department = counts_by(&input.official_rules, |rule| {
        nullish_property_key(rule.get("guideDepartment"), "unknown")
    });
    let counts_by_source_kind = counts_by(&all_rules, |rule| property_key(rule.get("sourceKind")));

    let mut report = Map::new();
    report.insert("generatedAt".to_string(), json!(input.generated_at));
    report.insert("sourceUrl".to_string(), json!(input.source_url));
    report.insert("status".to_string(), json!(input.status));
    report.insert("finalUrl".to_string(), json!(input.final_url));
    report.insert("contentType".to_string(), input.content_type.clone());
    report.insert("snapshotPath".to_string(), json!(input.snapshot_path));
    report.insert("snapshotSha256".to_string(), json!(input.snapshot_sha256));
    report.insert("departmentCount".to_string(), json!(department_count));
    report.insert("rowCount".to_string(), json!(input.rows.len()));
    report.insert(
        "officialRuleCount".to_string(),
        json!(input.official_rules.len()),
    );
    report.insert(
        "derivedRuleCount".to_string(),
        json!(input.derived_rules.len()),
    );
    report.insert("ruleCount".to_string(), json!(all_rules.len()));
    report.insert(
        "rulesWithSourceCourseSets".to_string(),
        json!(rules_with_source_course_sets),
    );
    report.insert(
        "rulesWithTargetCourseCodes".to_string(),
        json!(rules_with_target_course_codes),
    );
    report.insert("countsByType".to_string(), Value::Object(counts_by_type));
    report.insert(
        "countsByStatus".to_string(),
        Value::Object(counts_by_status),
    );
    report.insert(
        "countsByAcceptanceCategory".to_string(),
        Value::Object(counts_by_acceptance_category),
    );
    report.insert(
        "countsByDepartment".to_string(),
        Value::Object(counts_by_department),
    );
    report.insert(
        "countsBySourceKind".to_string(),
        Value::Object(counts_by_source_kind),
    );
    report.insert(
        "sampleRules".to_string(),
        Value::Array(all_rules.into_iter().take(20).collect()),
    );
    Value::Object(report)
}

pub fn render_equivalency_guide_report_markdown(report: &Value) -> String {
    let mut lines = vec![
        "# Transfer Planner Equivalency Guide Parse".to_string(),
        String::new(),
        format!("Generated at: {}", report_string(report, "generatedAt")),
        format!("Source: {}", report_string(report, "sourceUrl")),
        format!("HTTP status: {}", report_string(report, "status")),
        format!("Final URL: {}", report_string(report, "finalUrl")),
        format!("Snapshot hash: {}", report_string(report, "snapshotSha256")),
        String::new(),
        "## Counts".to_string(),
        String::new(),
        format!(
            "- Departments: {}",
            report_string(report, "departmentCount")
        ),
        format!(
            "- Official guide rows parsed: {}",
            report_string(report, "rowCount")
        ),
        format!("- Generated rules: {}", report_string(report, "ruleCount")),
        format!(
            "- Source-course-backed rules: {}",
            report_string(report, "rulesWithSourceCourseSets")
        ),
        format!(
            "- Target-course-backed rules: {}",
            report_string(report, "rulesWithTargetCourseCodes")
        ),
        String::new(),
        "## Rule Types".to_string(),
        String::new(),
    ];
    append_sorted_counts(&mut lines, report.get("countsByType"));
    lines.extend([String::new(), "## Rule Status".to_string(), String::new()]);
    append_sorted_counts(&mut lines, report.get("countsByStatus"));
    lines.extend([String::new(), "## Departments".to_string(), String::new()]);
    append_sorted_counts(&mut lines, report.get("countsByDepartment"));
    lines.push(String::new());
    format!("{}\n", lines.join("\n"))
}

fn counts_by(rules: &[Value], key: impl Fn(&Value) -> String) -> Map<String, Value> {
    let mut counts = Map::new();
    for rule in rules {
        let name = key(rule);
        let count = counts.get(&name).and_then(Value::as_u64).unwrap_or(0) + 1;
        counts.insert(name, json!(count));
    }
    counts
}

fn unique_row_departments(rows: &[Value]) -> usize {
    let mut departments = Vec::<Option<Value>>::new();
    for row in rows {
        let department = row.get("department").cloned();
        if !departments.contains(&department) {
            departments.push(department);
        }
    }
    departments.len()
}

fn append_sorted_counts(lines: &mut Vec<String>, counts: Option<&Value>) {
    let Some(counts) = counts.and_then(Value::as_object) else {
        return;
    };
    let mut entries = counts.iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(right.0));
    lines.extend(
        entries
            .into_iter()
            .map(|(key, count)| format!("- {key}: {count}")),
    );
}

fn report_string(report: &Value, key: &str) -> String {
    report
        .get(key)
        .map(|value| property_key(Some(value)))
        .unwrap_or_else(|| "undefined".to_string())
}

fn property_key(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(Value::String(value)) => value.clone(),
        Some(value) => value.to_string(),
    }
}

fn nullish_property_key(value: Option<&Value>, fallback: &str) -> String {
    match value {
        None | Some(Value::Null) => fallback.to_string(),
        Some(value) => property_key(Some(value)),
    }
}

fn string_field(value: &Value, key: &str) -> String {
    property_key(value.get(key))
}

#[cfg(test)]
mod tests {
    use super::{build_equivalency_guide_report, render_equivalency_guide_report_markdown};
    use serde_json::json;

    #[test]
    fn builds_counts_and_report_markdown_deterministically() {
        let input = serde_json::from_value(json!({
            "generatedAt": "2026-01-02T03:04:05.000Z",
            "sourceUrl": "https://example.test/guide",
            "status": 200,
            "finalUrl": "https://example.test/final",
            "contentType": "text/html",
            "snapshotPath": ".tmp/guide.html",
            "snapshotSha256": "abc123",
            "rows": [{"department":"Chemistry"}, {"department":"Biology"}, {"department":"Chemistry"}],
            "officialRules": [
                {"id":"rule:b","type":"sequence","ruleStatus":"active","acceptanceCategory":"preferred","guideDepartment":"Chemistry","sourceKind":"guide","sourceCourseSets":[["CHEM 2"]],"targetCourseCodes":["CHEM 3"]},
                {"id":"rule:a","type":"direct-course","ruleStatus":null,"acceptanceCategory":"accepted","guideDepartment":null,"sourceKind":"guide","sourceCourseSets":[],"targetCourseCodes":[]}
            ],
            "derivedRules": [{"id":"derived:1","type":"chain-rule","ruleStatus":"legacy","acceptanceCategory":"accepted-with-warning","sourceKind":"derived","sourceCourseSets":[["CHEM 2"]],"targetCourseCodes":["CHEM 3"]}]
        })).unwrap();
        let report = build_equivalency_guide_report(&input);
        assert_eq!(report["departmentCount"], 2);
        assert_eq!(report["rowCount"], 3);
        assert_eq!(report["officialRuleCount"], 2);
        assert_eq!(report["derivedRuleCount"], 1);
        assert_eq!(report["ruleCount"], 3);
        assert_eq!(report["rulesWithSourceCourseSets"], 2);
        assert_eq!(report["rulesWithTargetCourseCodes"], 2);
        assert_eq!(report["countsByType"]["direct-course"], 1);
        assert_eq!(report["countsByStatus"]["unknown"], 1);
        assert_eq!(report["countsByDepartment"]["unknown"], 1);
        assert_eq!(report["sampleRules"][0]["id"], "derived:1");
        assert_eq!(
            render_equivalency_guide_report_markdown(&report),
            concat!(
                "# Transfer Planner Equivalency Guide Parse\n\n",
                "Generated at: 2026-01-02T03:04:05.000Z\n",
                "Source: https://example.test/guide\n",
                "HTTP status: 200\n",
                "Final URL: https://example.test/final\n",
                "Snapshot hash: abc123\n\n",
                "## Counts\n\n",
                "- Departments: 2\n",
                "- Official guide rows parsed: 3\n",
                "- Generated rules: 3\n",
                "- Source-course-backed rules: 2\n",
                "- Target-course-backed rules: 2\n\n",
                "## Rule Types\n\n",
                "- chain-rule: 1\n",
                "- direct-course: 1\n",
                "- sequence: 1\n\n",
                "## Rule Status\n\n",
                "- active: 1\n",
                "- legacy: 1\n",
                "- unknown: 1\n\n",
                "## Departments\n\n",
                "- Chemistry: 1\n",
                "- unknown: 1\n\n"
            )
        );
    }
}
