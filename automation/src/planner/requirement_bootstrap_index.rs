use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct RequirementBootstrapPlanIndex {
    plan_titles_by_id: HashMap<String, String>,
    pathway_counts_by_plan_id: HashMap<String, usize>,
}

impl RequirementBootstrapPlanIndex {
    pub fn from_plans(plans: &[Value]) -> Self {
        let mut index = Self::default();
        for plan in plans {
            let Some(plan_id) = plan.get("id").and_then(Value::as_str) else {
                continue;
            };
            let title = plan
                .get("title")
                .filter(|value| !value.is_null())
                .map(js_string)
                .unwrap_or_default();
            index
                .plan_titles_by_id
                .insert(plan_id.to_string(), title);
            let pathway_count = plan
                .get("pathways")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            index
                .pathway_counts_by_plan_id
                .insert(plan_id.to_string(), pathway_count);
        }
        index
    }

    pub fn plan_titles_by_id(&self) -> &HashMap<String, String> {
        &self.plan_titles_by_id
    }

    pub fn pathway_count(&self, plan_id: &str) -> usize {
        self.pathway_counts_by_plan_id
            .get(plan_id)
            .copied()
            .unwrap_or_default()
    }

    pub fn materializable_parser_pathway_id(&self, entry: &Value) -> Option<String> {
        let pathway_id = entry
            .get("pathwayId")
            .filter(|value| !value.is_null())
            .map(js_string)
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if pathway_id.is_empty() {
            return None;
        }
        let plan_id = entry
            .get("planId")
            .filter(|value| !value.is_null())
            .map(js_string)
            .unwrap_or_default();
        (self.pathway_count(&plan_id) > 0).then_some(pathway_id)
    }
}

fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(|value| if value.is_null() { String::new() } else { js_string(value) })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}
