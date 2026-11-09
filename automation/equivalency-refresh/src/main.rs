use serde::Serialize;
use serde::de::DeserializeOwned;
use std::env;
use std::io::{self, Read, Write};
use std::process::ExitCode;

#[path = "../../src/planner/equivalency_catalog.rs"]
mod equivalency_catalog;
#[path = "../../src/planner/equivalency_derived.rs"]
mod equivalency_derived;
#[path = "../../src/planner/equivalency_output.rs"]
mod equivalency_output;
#[path = "../../src/planner/equivalency_report.rs"]
mod equivalency_report;
#[allow(dead_code)]
#[path = "../../src/planner/equivalency_rows.rs"]
mod equivalency_rows;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("equivalency refresh failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let command = env::args()
        .nth(1)
        .ok_or_else(|| "missing equivalency-refresh command".to_string())?;
    match command.as_str() {
        "parse-equivalency-guide-rows" => {
            let input: equivalency_rows::EquivalencyGuideParseInput = read_json()?;
            let rows = equivalency_rows::parse_equivalency_guide_rows_with_titles(
                &input.html,
                &input.grc_course_titles,
            );
            write_json(&rows)
        }
        "synthesize-uw-grc-derived-rules" => {
            let input: equivalency_derived::DerivedRuleSynthesisInput = read_json()?;
            let rules = equivalency_derived::synthesize_equivalency_rules(&input)?;
            write_json(&rules)
        }
        "build-equivalency-guide-report" => {
            let input: equivalency_report::EquivalencyGuideReportInput = read_json()?;
            let report = equivalency_report::build_equivalency_guide_report(&input);
            let markdown = equivalency_report::render_equivalency_guide_report_markdown(&report);
            write_json(&serde_json::json!({ "report": report, "markdown": markdown }))
        }
        "render-equivalency-guide-typescript" => {
            let input: equivalency_output::EquivalencyRulesOutputInput = read_json()?;
            let output = equivalency_output::render_equivalency_guide_typescript(&input)?;
            write_text(&output)
        }
        "render-transfer-equivalency-catalog-typescript" => {
            let input: equivalency_output::EquivalencyRulesOutputInput = read_json()?;
            let output =
                equivalency_catalog::render_transfer_equivalency_catalog_typescript(&input)?;
            write_text(&output)
        }
        "--help" | "-h" | "help" => {
            println!(
                "Commands: parse-equivalency-guide-rows, synthesize-uw-grc-derived-rules, build-equivalency-guide-report, render-equivalency-guide-typescript, render-transfer-equivalency-catalog-typescript"
            );
            Ok(())
        }
        other => Err(format!("unknown command {other:?}")),
    }
}

fn read_json<T: DeserializeOwned>() -> Result<T, String> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|error| format!("could not read JSON input from stdin: {error}"))?;
    serde_json::from_str(&input).map_err(|error| format!("could not decode JSON input: {error}"))
}

fn write_json<T: Serialize>(value: &T) -> Result<(), String> {
    let encoded = serde_json::to_vec(value)
        .map_err(|error| format!("could not encode JSON output: {error}"))?;
    io::stdout()
        .write_all(&encoded)
        .map_err(|error| format!("could not write JSON output: {error}"))
}

fn write_text(value: &str) -> Result<(), String> {
    io::stdout()
        .write_all(value.as_bytes())
        .map_err(|error| format!("could not write text output: {error}"))
}
