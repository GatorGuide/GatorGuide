use gator_guide_automation::{OpportunityCategory, build_http_client, build_report};
use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("automation failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let Some(command) = args.first().map(String::as_str) else {
        print_usage();
        return Err("missing command".to_string());
    };
    if command == "--help" || command == "-h" || command == "help" {
        print_usage();
        return Ok(());
    }

    let repository_root = repository_root()?;
    match command {
        "refresh-planner" => run_planner_refresh(&repository_root),
        "collect-opportunities" => collect_opportunity_candidates(&repository_root, &args[1..]),
        other => Err(format!(
            "unknown command {other:?}; run with --help for usage"
        )),
    }
}

fn run_planner_refresh(repository_root: &Path) -> Result<(), String> {
    let source_root = repository_root.join("source");
    let script = source_root.join("scripts/planner/refresh-transfer-planner-sources.cjs");
    if !script.is_file() {
        return Err(format!(
            "planner refresh script not found: {}",
            script.display()
        ));
    }
    println!("Running deterministic planner refresh through the maintained source adapters…");
    let status = Command::new("node")
        .arg(script)
        .arg("--skip-auto-repair")
        .current_dir(&source_root)
        .status()
        .map_err(|error| format!("could not start Node.js planner parser: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("planner refresh exited with status {status}"))
    }
}

fn collect_opportunity_candidates(repository_root: &Path, args: &[String]) -> Result<(), String> {
    let mut category = None;
    let mut catalog = repository_root.join("source/data/starter-opportunities.json");
    let mut output = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--category" => {
                index += 1;
                let value = args.get(index).ok_or("--category requires a value")?;
                category = Some(OpportunityCategory::parse(value)?);
            }
            "--catalog" => {
                index += 1;
                let value = args.get(index).ok_or("--catalog requires a path")?;
                catalog = resolve_from_root(repository_root, value);
            }
            "--output" => {
                index += 1;
                let value = args.get(index).ok_or("--output requires a path")?;
                output = Some(resolve_from_root(repository_root, value));
            }
            other => return Err(format!("unexpected argument {other:?}")),
        }
        index += 1;
    }

    let category = category.ok_or("collect-opportunities requires --category")?;
    let output = output.ok_or("collect-opportunities requires --output")?;
    let catalog_text = fs::read_to_string(&catalog)
        .map_err(|error| format!("could not read catalog {}: {error}", catalog.display()))?;
    let catalog_json: Value = serde_json::from_str(&catalog_text)
        .map_err(|error| format!("catalog {} is invalid JSON: {error}", catalog.display()))?;
    let client = build_http_client()?;
    let report = build_report(&catalog_json, category, &client)?;
    let encoded = serde_json::to_vec_pretty(&report)
        .map_err(|error| format!("could not serialize candidate report: {error}"))?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "could not create output directory {}: {error}",
                parent.display()
            )
        })?;
    }
    fs::write(&output, encoded).map_err(|error| {
        format!(
            "could not write candidate report {}: {error}",
            output.display()
        )
    })?;

    let failed = report
        .sources
        .iter()
        .filter(|source| source.state != "checked")
        .count();
    let candidates = report
        .sources
        .iter()
        .map(|source| source.structured_candidates.len() + source.candidate_links.len())
        .sum::<usize>();
    println!(
        "Reviewed {} {} source pages; {candidates} deterministic candidate facts/links; {failed} source fetches need review.",
        report.source_count,
        match category {
            OpportunityCategory::Scholarships => "scholarship",
            OpportunityCategory::Internships => "internship",
        }
    );
    println!("Review-only report: {}", output.display());
    Ok(())
}

fn repository_root() -> Result<PathBuf, String> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "automation crate is not located below a repository root".to_string())
}

fn resolve_from_root(root: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn print_usage() {
    println!(
        "GatorGuide deterministic automation\n\n\
         Usage:\n\
           cargo run --manifest-path automation/Cargo.toml -- refresh-planner\n\
           cargo run --manifest-path automation/Cargo.toml -- collect-opportunities --category scholarships --output <path>\n\n\
         refresh-planner runs the maintained, deterministic planner source pipeline.\n\
         collect-opportunities fetches only source URLs already listed in starter-opportunities.json and emits a review-only report; it never edits the catalog."
    );
}
