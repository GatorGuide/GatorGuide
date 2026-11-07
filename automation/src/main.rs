use gator_guide_automation::{
    OpportunityCategory, build_http_client, build_report,
    planner::{
        GrcAnnualScheduleCandidatesInput, GrcAnnualScheduleFilterInput,
        GrcCatalogArchiveEntriesInput, GrcCourseCatalogInput, GrcCourseDetailsInput,
        GrcCurrentCatalogDetailsInput, detect_catalog_page_numbers, discover_grc_public_materials,
        filter_relevant_annual_schedule_values, parse_annual_schedule_candidates,
        parse_catalog_archive_entries, parse_current_catalog_details, parse_grc_course_catalog,
        parse_grc_course_details,
    },
};
use serde_json::Value;
use std::env;
use std::fs;
use std::io::{Read, Write};
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
        "discover-grc-materials" => discover_grc_materials(&repository_root, &args[1..]),
        "parse-grc-catalog-page-numbers" => parse_grc_catalog_page_numbers_command(&args[1..]),
        "parse-grc-course-catalog" => parse_grc_course_catalog_command(&args[1..]),
        "parse-grc-course-details" => parse_grc_course_details_command(&args[1..]),
        "parse-grc-annual-schedule-candidates" => {
            parse_grc_annual_schedule_candidates_command(&args[1..])
        }
        "filter-grc-annual-schedules" => filter_grc_annual_schedules_command(&args[1..]),
        "parse-grc-catalog-archive-entries" => {
            parse_grc_catalog_archive_entries_command(&args[1..])
        }
        "parse-grc-current-catalog-details" => {
            parse_grc_current_catalog_details_command(&args[1..])
        }
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

fn parse_grc_catalog_page_numbers_command(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("unexpected arguments: {}", args.join(" ")));
    }
    let mut input_json = String::new();
    std::io::stdin()
        .read_to_string(&mut input_json)
        .map_err(|error| {
            format!("could not read GRC catalog pagination input from stdin: {error}")
        })?;
    let input: Value = serde_json::from_str(&input_json)
        .map_err(|error| format!("could not decode GRC catalog pagination input: {error}"))?;
    let html = input
        .get("html")
        .and_then(Value::as_str)
        .ok_or_else(|| "GRC catalog pagination input is missing html".to_string())?;
    let page_numbers = detect_catalog_page_numbers(html);
    let encoded = serde_json::to_vec(&page_numbers)
        .map_err(|error| format!("could not encode GRC catalog page numbers: {error}"))?;
    std::io::stdout()
        .write_all(&encoded)
        .map_err(|error| format!("could not write GRC catalog page numbers: {error}"))
}

fn parse_grc_course_catalog_command(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("unexpected arguments: {}", args.join(" ")));
    }
    let mut input_json = String::new();
    std::io::stdin()
        .read_to_string(&mut input_json)
        .map_err(|error| format!("could not read GRC course-catalog input from stdin: {error}"))?;
    let input: GrcCourseCatalogInput = serde_json::from_str(&input_json)
        .map_err(|error| format!("could not decode GRC course-catalog input: {error}"))?;
    let sections = parse_grc_course_catalog(&input.html);
    let encoded = serde_json::to_vec(&sections)
        .map_err(|error| format!("could not encode GRC course catalog sections: {error}"))?;
    std::io::stdout()
        .write_all(&encoded)
        .map_err(|error| format!("could not write GRC course catalog sections: {error}"))
}

fn parse_grc_course_details_command(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("unexpected arguments: {}", args.join(" ")));
    }
    let mut input_json = String::new();
    std::io::stdin()
        .read_to_string(&mut input_json)
        .map_err(|error| format!("could not read GRC course-details input from stdin: {error}"))?;
    let input: GrcCourseDetailsInput = serde_json::from_str(&input_json)
        .map_err(|error| format!("could not decode GRC course-details input: {error}"))?;
    let details = parse_grc_course_details(&input.raw_bodies);
    let encoded = serde_json::to_vec(&details)
        .map_err(|error| format!("could not encode GRC course details: {error}"))?;
    std::io::stdout()
        .write_all(&encoded)
        .map_err(|error| format!("could not write GRC course details: {error}"))
}

fn parse_grc_annual_schedule_candidates_command(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("unexpected arguments: {}", args.join(" ")));
    }
    let mut input_json = String::new();
    std::io::stdin()
        .read_to_string(&mut input_json)
        .map_err(|error| format!("could not read GRC schedule-page input from stdin: {error}"))?;
    let input: GrcAnnualScheduleCandidatesInput = serde_json::from_str(&input_json)
        .map_err(|error| format!("could not decode GRC schedule-page input: {error}"))?;
    let candidates = parse_annual_schedule_candidates(&input.html, &input.base_url)?;
    let encoded = serde_json::to_vec(&candidates)
        .map_err(|error| format!("could not encode GRC annual-schedule candidates: {error}"))?;
    std::io::stdout()
        .write_all(&encoded)
        .map_err(|error| format!("could not write GRC annual-schedule candidates: {error}"))
}

fn filter_grc_annual_schedules_command(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("unexpected arguments: {}", args.join(" ")));
    }
    let mut input_json = String::new();
    std::io::stdin()
        .read_to_string(&mut input_json)
        .map_err(|error| {
            format!("could not read annual-schedule filter input from stdin: {error}")
        })?;
    let input: GrcAnnualScheduleFilterInput = serde_json::from_str(&input_json)
        .map_err(|error| format!("could not decode annual-schedule filter input: {error}"))?;
    let schedules = filter_relevant_annual_schedule_values(
        input.annual_schedules,
        &input.current_catalog_year_label,
    );
    let encoded = serde_json::to_vec(&schedules)
        .map_err(|error| format!("could not encode filtered annual schedules: {error}"))?;
    std::io::stdout()
        .write_all(&encoded)
        .map_err(|error| format!("could not write filtered annual schedules: {error}"))
}

fn parse_grc_catalog_archive_entries_command(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("unexpected arguments: {}", args.join(" ")));
    }
    let mut input_json = String::new();
    std::io::stdin()
        .read_to_string(&mut input_json)
        .map_err(|error| format!("could not read GRC catalog-archive input from stdin: {error}"))?;
    let input: GrcCatalogArchiveEntriesInput = serde_json::from_str(&input_json)
        .map_err(|error| format!("could not decode GRC catalog-archive input: {error}"))?;
    let entries = parse_catalog_archive_entries(&input.html, &input.base_url)?;
    let encoded = serde_json::to_vec(&entries)
        .map_err(|error| format!("could not encode GRC catalog-archive entries: {error}"))?;
    std::io::stdout()
        .write_all(&encoded)
        .map_err(|error| format!("could not write GRC catalog-archive entries: {error}"))
}

fn parse_grc_current_catalog_details_command(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("unexpected arguments: {}", args.join(" ")));
    }
    let mut input_json = String::new();
    std::io::stdin()
        .read_to_string(&mut input_json)
        .map_err(|error| format!("could not read GRC current-catalog input from stdin: {error}"))?;
    let input: GrcCurrentCatalogDetailsInput = serde_json::from_str(&input_json)
        .map_err(|error| format!("could not decode GRC current-catalog input: {error}"))?;
    let details = parse_current_catalog_details(
        &input.html,
        &input.catalog_root_url,
        &input.expected_catalog_year_label,
    )?;
    let encoded = serde_json::to_vec(&details)
        .map_err(|error| format!("could not encode GRC current-catalog details: {error}"))?;
    std::io::stdout()
        .write_all(&encoded)
        .map_err(|error| format!("could not write GRC current-catalog details: {error}"))
}

fn discover_grc_materials(repository_root: &Path, args: &[String]) -> Result<(), String> {
    let mut force_refresh = false;
    let mut allow_snapshot_fallback = true;
    let mut cache_only = env::var("GATORGUIDE_PLANNER_CACHE_ONLY").as_deref() == Ok("1");
    for argument in args {
        match argument.as_str() {
            "--refresh" => force_refresh = true,
            "--no-snapshot-fallback" => allow_snapshot_fallback = false,
            "--cache-only" => cache_only = true,
            other => return Err(format!("unexpected argument {other:?}")),
        }
    }

    let materials = discover_grc_public_materials(
        repository_root,
        force_refresh,
        allow_snapshot_fallback,
        cache_only,
    )?;
    println!(
        "Discovered {} Green River annual schedules and {} catalog archive entries.",
        materials.annual_schedules.len(),
        materials.catalog_entries.len()
    );
    println!("Current catalog year: {}", materials.current_catalog.label);
    println!(
        "Discovery report: {}",
        repository_root
            .join("source/.tmp/reports/transfer-planner-grc-public-materials.json")
            .display()
    );
    Ok(())
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
           cargo run --manifest-path automation/Cargo.toml -- discover-grc-materials [--refresh] [--cache-only]\n\
           cargo run --manifest-path automation/Cargo.toml -- parse-grc-catalog-page-numbers < pager-input.json\n\
           cargo run --manifest-path automation/Cargo.toml -- parse-grc-course-catalog < catalog-page.json\n\
           cargo run --manifest-path automation/Cargo.toml -- parse-grc-course-details < course-bodies.json\n\
           cargo run --manifest-path automation/Cargo.toml -- parse-grc-annual-schedule-candidates < schedule-page.json\n\
           cargo run --manifest-path automation/Cargo.toml -- filter-grc-annual-schedules < schedules.json\n\
           cargo run --manifest-path automation/Cargo.toml -- parse-grc-catalog-archive-entries < catalog-archive.json\n\
           cargo run --manifest-path automation/Cargo.toml -- parse-grc-current-catalog-details < current-catalog.json\n\
           cargo run --manifest-path automation/Cargo.toml -- collect-opportunities --category scholarships --output <path>\n\n\
         refresh-planner runs the maintained, deterministic planner source pipeline.\n\
         discover-grc-materials parses Green River's official catalog archive and annual-schedule links.\n\
         parse-grc-course-catalog parses an {{html}} page into ordered course-heading and detail sections.\n\
         parse-grc-course-details parses raw course bodies into credits, descriptions, requirement sections, and general-education tags.\n\
         collect-opportunities fetches only source URLs already listed in starter-opportunities.json and emits a review-only report; it never edits the catalog."
    );
}
