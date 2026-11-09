use gator_guide_automation::{
    OpportunityCategory, build_http_client, build_report,
    planner::{
        DerivedRuleSynthesisInput, EquivalencyGuideParseInput, EquivalencyGuideReportInput,
        EquivalencyRulesOutputInput, GrcAnnualScheduleCandidatesInput,
        GrcAnnualScheduleFilterInput, GrcCatalogArchiveEntriesInput, GrcCourseCatalogInput,
        GrcCourseDetailsInput, GrcCurrentCatalogDetailsInput, SequenceCourseCodeInput,
        SequenceInlineSegmentsInput, build_equivalency_guide_report,
        build_source_pipeline_core_execution_from_json, detect_catalog_page_numbers,
        discover_grc_public_materials, expand_sequence_course_codes,
        filter_relevant_annual_schedule_values, parse_annual_schedule_candidates,
        parse_catalog_archive_entries, parse_current_catalog_details,
        parse_equivalency_guide_rows_with_titles, parse_grc_course_catalog,
        parse_grc_course_details, parse_target_plan_ids, refresh_deadlines, refresh_uw_catalog,
        render_equivalency_guide_report_markdown, render_equivalency_guide_typescript,
        render_transfer_equivalency_catalog_typescript, run_build_source_fingerprints,
        run_owner_source_link_probe, run_planner_refresh, run_publish_readiness,
        run_source_backed_coverage_audit, run_spot_check_major, split_inline_sequence_segments,
        synthesize_equivalency_rules,
    },
};
use serde_json::Value;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args
        .first()
        .is_some_and(|command| command == "verify-source-pipeline-core")
    {
        return verify_source_pipeline_core_command(&args[1..]);
    }
    if args
        .first()
        .is_some_and(|command| command == "spot-check-planner-major")
    {
        return match repository_root().and_then(|root| run_spot_check_major(&root, &args[1..])) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(error) => {
                eprintln!("Error: {error}");
                ExitCode::FAILURE
            }
        };
    }
    if args
        .first()
        .is_some_and(|command| command == "probe-owner-source-links")
    {
        return match repository_root()
            .and_then(|root| run_owner_source_link_probe(&root, &args[1..]))
        {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(error) => {
                eprintln!("Error: {error}");
                ExitCode::FAILURE
            }
        };
    }
    if args
        .first()
        .is_some_and(|command| command == "audit-planner-publish-readiness")
    {
        return match repository_root().and_then(|root| run_publish_readiness(&root, &args[1..])) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(error) => {
                eprintln!("automation failed: {error}");
                ExitCode::FAILURE
            }
        };
    }
    if args
        .first()
        .is_some_and(|command| command == "audit-source-backed-coverage")
    {
        return match repository_root()
            .and_then(|root| run_source_backed_coverage_audit(&root, &args[1..]))
        {
            Ok(0) => ExitCode::SUCCESS,
            Ok(_) => ExitCode::FAILURE,
            Err(error) => {
                eprintln!("automation failed: {error}");
                ExitCode::FAILURE
            }
        };
    }
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("automation failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn verify_source_pipeline_core_command(args: &[String]) -> ExitCode {
    let target_plan_ids = match parse_target_plan_ids(args) {
        Ok(target_plan_ids) => target_plan_ids,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    if target_plan_ids.len() > 1 {
        eprintln!(
            "Source pipeline validation supports one target plan at a time; received {}.",
            target_plan_ids.join(", ")
        );
        return ExitCode::FAILURE;
    }
    let mut input_json = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut input_json) {
        eprintln!("could not read source-pipeline input from stdin: {error}");
        return ExitCode::FAILURE;
    }
    let mut input: Value = match serde_json::from_str(&input_json) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("could not decode source-pipeline input: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(target_plan_id) = target_plan_ids.first() {
        let Some(object) = input.as_object_mut() else {
            eprintln!("source-pipeline input must be an object");
            return ExitCode::FAILURE;
        };
        object.insert(
            "validationTargetPlanId".to_string(),
            Value::String(target_plan_id.clone()),
        );
    }
    let execution = match build_source_pipeline_core_execution_from_json(&input) {
        Ok(execution) => execution,
        Err(error) => {
            eprintln!("could not validate source-pipeline input: {error}");
            return ExitCode::FAILURE;
        }
    };
    let exit_code = execution.exit_code;
    let output = serde_json::json!({
        "report": execution.report,
        "json": execution.artifacts.json,
        "markdown": execution.artifacts.markdown,
        "exitCode": exit_code
    });
    let encoded = match serde_json::to_vec(&output) {
        Ok(encoded) => encoded,
        Err(error) => {
            eprintln!("could not encode source-pipeline output: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = std::io::stdout().write_all(&encoded) {
        eprintln!("could not write source-pipeline output: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::from(u8::try_from(exit_code).unwrap_or(1))
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
        "refresh-planner" => {
            let mut refresh_args = args[1..].to_vec();
            if !refresh_args
                .iter()
                .any(|argument| argument == "--skip-auto-repair")
            {
                refresh_args.push("--skip-auto-repair".to_string());
            }
            run_planner_refresh(&repository_root, &refresh_args)
        }
        "build-source-fingerprints" => {
            run_build_source_fingerprints(&repository_root, &args[1..]).map(|_| ())
        }
        "discover-grc-materials" => discover_grc_materials(&repository_root, &args[1..]),
        "refresh-deadlines" => refresh_deadlines_command(&repository_root, &args[1..]),
        "ingest-uw-catalog" => ingest_uw_catalog_command(&repository_root, &args[1..]),
        "parse-equivalency-guide-rows" => parse_equivalency_guide_rows_command(&args[1..]),
        "parse-grc-catalog-page-numbers" => parse_grc_catalog_page_numbers_command(&args[1..]),
        "parse-grc-course-catalog" => parse_grc_course_catalog_command(&args[1..]),
        "parse-grc-course-details" => parse_grc_course_details_command(&args[1..]),
        "parse-sequence-inline-segments" => parse_sequence_inline_segments_command(&args[1..]),
        "parse-sequence-course-codes" => parse_sequence_course_codes_command(&args[1..]),
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
        "render-equivalency-guide-typescript" => {
            render_equivalency_guide_typescript_command(&args[1..])
        }
        "render-transfer-equivalency-catalog-typescript" => {
            render_transfer_equivalency_catalog_typescript_command(&args[1..])
        }
        "synthesize-uw-grc-derived-rules" => synthesize_uw_grc_derived_rules_command(&args[1..]),
        "build-equivalency-guide-report" => build_equivalency_guide_report_command(&args[1..]),
        "collect-opportunities" => collect_opportunity_candidates(&repository_root, &args[1..]),
        other => Err(format!(
            "unknown command {other:?}; run with --help for usage"
        )),
    }
}

fn parse_equivalency_guide_rows_command(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("unexpected arguments: {}", args.join(" ")));
    }
    let mut input_json = String::new();
    std::io::stdin()
        .read_to_string(&mut input_json)
        .map_err(|error| {
            format!("could not read equivalency-guide parser input from stdin: {error}")
        })?;
    let input: EquivalencyGuideParseInput = serde_json::from_str(&input_json)
        .map_err(|error| format!("could not decode equivalency-guide parser input: {error}"))?;
    let rows = parse_equivalency_guide_rows_with_titles(&input.html, &input.grc_course_titles);
    let encoded = serde_json::to_vec(&rows)
        .map_err(|error| format!("could not encode equivalency-guide rows: {error}"))?;
    std::io::stdout()
        .write_all(&encoded)
        .map_err(|error| format!("could not write equivalency-guide rows: {error}"))
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

fn parse_sequence_inline_segments_command(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("unexpected arguments: {}", args.join(" ")));
    }
    let mut input_json = String::new();
    std::io::stdin()
        .read_to_string(&mut input_json)
        .map_err(|error| format!("could not read sequence parser input from stdin: {error}"))?;
    let input: SequenceInlineSegmentsInput = serde_json::from_str(&input_json)
        .map_err(|error| format!("could not decode sequence parser input: {error}"))?;
    let segments = split_inline_sequence_segments(&input.texts);
    let encoded = serde_json::to_vec(&segments)
        .map_err(|error| format!("could not encode sequence parser output: {error}"))?;
    std::io::stdout()
        .write_all(&encoded)
        .map_err(|error| format!("could not write sequence parser output: {error}"))
}

fn parse_sequence_course_codes_command(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("unexpected arguments: {}", args.join(" ")));
    }
    let mut input_json = String::new();
    std::io::stdin()
        .read_to_string(&mut input_json)
        .map_err(|error| {
            format!("could not read sequence course-code input from stdin: {error}")
        })?;
    let input: SequenceCourseCodeInput = serde_json::from_str(&input_json)
        .map_err(|error| format!("could not decode sequence course-code input: {error}"))?;
    let course_codes = expand_sequence_course_codes(&input.rows);
    let encoded = serde_json::to_vec(&course_codes)
        .map_err(|error| format!("could not encode sequence course-code output: {error}"))?;
    std::io::stdout()
        .write_all(&encoded)
        .map_err(|error| format!("could not write sequence course-code output: {error}"))
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

fn render_equivalency_guide_typescript_command(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("unexpected arguments: {}", args.join(" ")));
    }
    let mut input_json = String::new();
    std::io::stdin()
        .read_to_string(&mut input_json)
        .map_err(|error| format!("could not read generated-rule input from stdin: {error}"))?;
    let input: EquivalencyRulesOutputInput = serde_json::from_str(&input_json)
        .map_err(|error| format!("could not decode generated-rule input: {error}"))?;
    let output = render_equivalency_guide_typescript(&input)?;
    std::io::stdout()
        .write_all(output.as_bytes())
        .map_err(|error| format!("could not write generated TypeScript: {error}"))
}

fn render_transfer_equivalency_catalog_typescript_command(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("unexpected arguments: {}", args.join(" ")));
    }
    let mut input_json = String::new();
    std::io::stdin()
        .read_to_string(&mut input_json)
        .map_err(|error| {
            format!("could not read transfer-equivalency catalog input from stdin: {error}")
        })?;
    let input: EquivalencyRulesOutputInput = serde_json::from_str(&input_json)
        .map_err(|error| format!("could not decode transfer-equivalency catalog input: {error}"))?;
    let output = render_transfer_equivalency_catalog_typescript(&input)?;
    std::io::stdout()
        .write_all(output.as_bytes())
        .map_err(|error| {
            format!("could not write transfer-equivalency catalog TypeScript: {error}")
        })
}

fn synthesize_uw_grc_derived_rules_command(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("unexpected arguments: {}", args.join(" ")));
    }
    let mut input_json = String::new();
    std::io::stdin()
        .read_to_string(&mut input_json)
        .map_err(|error| format!("could not read derived-rule input from stdin: {error}"))?;
    let input: DerivedRuleSynthesisInput = serde_json::from_str(&input_json)
        .map_err(|error| format!("could not decode derived-rule input: {error}"))?;
    let rules = synthesize_equivalency_rules(&input)?;
    let encoded = serde_json::to_vec(&rules)
        .map_err(|error| format!("could not encode derived rules: {error}"))?;
    std::io::stdout()
        .write_all(&encoded)
        .map_err(|error| format!("could not write derived rules: {error}"))
}

fn build_equivalency_guide_report_command(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("unexpected arguments: {}", args.join(" ")));
    }
    let mut input_json = String::new();
    std::io::stdin()
        .read_to_string(&mut input_json)
        .map_err(|error| format!("could not read equivalency-report input from stdin: {error}"))?;
    let input: EquivalencyGuideReportInput = serde_json::from_str(&input_json)
        .map_err(|error| format!("could not decode equivalency-report input: {error}"))?;
    let report = build_equivalency_guide_report(&input);
    let markdown = render_equivalency_guide_report_markdown(&report);
    let encoded = serde_json::to_vec(&serde_json::json!({"report": report, "markdown": markdown}))
        .map_err(|error| format!("could not encode equivalency report: {error}"))?;
    std::io::stdout()
        .write_all(&encoded)
        .map_err(|error| format!("could not write equivalency report: {error}"))
}

fn refresh_deadlines_command(repository_root: &Path, args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(format!("unexpected arguments: {}", args.join(" ")));
    }
    let report = refresh_deadlines(repository_root)?;
    println!(
        "Refreshed {} official deadlines: {} created, {} updated, {} unchanged.",
        report.parsed_count,
        report.created.len(),
        report.updated.len(),
        report.unchanged.len()
    );
    println!("Updated opportunity catalog: {}", report.output_path);
    println!("Refresh report: {}", report.report_path);
    Ok(())
}

fn ingest_uw_catalog_command(repository_root: &Path, args: &[String]) -> Result<(), String> {
    let mut cache_only = env::var("GATORGUIDE_PLANNER_CACHE_ONLY").as_deref() == Ok("1");
    for argument in args {
        match argument.as_str() {
            "--cache-only" => cache_only = true,
            other => return Err(format!("unexpected argument {other:?}")),
        }
    }
    let report = refresh_uw_catalog(repository_root, cache_only)?;
    println!(
        "Parsed {} of {} UW planner-relevant catalog courses across {} campuses.",
        report.course_count,
        report.relevant_course_count,
        report.campuses.len()
    );
    println!(
        "UW catalog reports: source/.tmp/reports/transfer-planner-uw-catalog-ingest.json and .md"
    );
    Ok(())
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
           cargo run --manifest-path automation/Cargo.toml -- build-source-fingerprints [--check-sources-first] [--parse-requirements-first]\n\
           cargo run --manifest-path automation/Cargo.toml -- probe-owner-source-links --target-plan-id <id> [checks]\n\
           cargo run --manifest-path automation/Cargo.toml -- spot-check-planner-major [--list | --commands | --target-plan-id <id> [checks]]\n\
           cargo run --manifest-path automation/Cargo.toml -- discover-grc-materials [--refresh] [--cache-only]\n\
           cargo run --manifest-path automation/Cargo.toml -- refresh-deadlines\n\
           cargo run --manifest-path automation/Cargo.toml -- ingest-uw-catalog [--cache-only]\n\
           cargo run --manifest-path automation/Cargo.toml -- parse-equivalency-guide-rows < parser-input.json\n\
           cargo run --manifest-path automation/Cargo.toml -- parse-grc-catalog-page-numbers < pager-input.json\n\
           cargo run --manifest-path automation/Cargo.toml -- parse-grc-course-catalog < catalog-page.json\n\
           cargo run --manifest-path automation/Cargo.toml -- parse-grc-course-details < course-bodies.json\n\
           cargo run --manifest-path automation/Cargo.toml -- parse-sequence-inline-segments < sequences.json\n\
           cargo run --manifest-path automation/Cargo.toml -- parse-sequence-course-codes < sequence-codes.json\n\
           cargo run --manifest-path automation/Cargo.toml -- parse-grc-annual-schedule-candidates < schedule-page.json\n\
           cargo run --manifest-path automation/Cargo.toml -- filter-grc-annual-schedules < schedules.json\n\
           cargo run --manifest-path automation/Cargo.toml -- parse-grc-catalog-archive-entries < catalog-archive.json\n\
           cargo run --manifest-path automation/Cargo.toml -- parse-grc-current-catalog-details < current-catalog.json\n\
           cargo run --manifest-path automation/Cargo.toml -- render-equivalency-guide-typescript < rules.json\n\
           cargo run --manifest-path automation/Cargo.toml -- render-transfer-equivalency-catalog-typescript < rules.json\n\
           cargo run --manifest-path automation/Cargo.toml -- synthesize-uw-grc-derived-rules < derived-input.json\n\
           cargo run --manifest-path automation/Cargo.toml -- build-equivalency-guide-report < report-input.json\n\
           cargo run --manifest-path automation/Cargo.toml -- collect-opportunities --category scholarships --output <path>\n\
           cargo run --manifest-path automation/Cargo.toml -- audit-planner-publish-readiness [--mode <mode>] [--run-checks]\n\
           cargo run --manifest-path automation/Cargo.toml -- audit-source-backed-coverage [--mapping-only | --generated-registry-only] [--report-only] [--target-plan-id <id>] [--protected-only] --context-stdin\n\
           cargo run --manifest-path automation/Cargo.toml -- verify-source-pipeline-core [--target-plan-id <id>] < validation-context.json\n\n\
         discover-grc-materials parses Green River's official catalog archive and annual-schedule links.\n\
         refresh-deadlines fetches the configured official deadline sources and updates the opportunity catalog.\n\
         build-source-fingerprints writes source and requirement fingerprints, classification reports, and the generated registry; missing prerequisite reports are produced by their maintained Node scripts.\n\
         probe-owner-source-links checks supplemental document links for a selected planner source owner; TypeScript planner registry data is loaded from the canonical source modules.\n\
         spot-check-planner-major lists planner majors, prints targeted commands, or runs selected existing Node checks from Rust orchestration.\n\
         ingest-uw-catalog fetches UW catalog pages and writes a deterministic source report.\n\
         parse-equivalency-guide-rows parses an {{html, grcCourseTitles}} JSON object on stdin and emits generated rules.\n\
         parse-grc-catalog-page-numbers parses an {{html}} pager fragment and emits a contiguous sorted page-number array.\n\
         parse-grc-course-catalog parses an {{html}} page into ordered course-heading and detail sections.\n\
         parse-grc-course-details parses a rawBodies array into credits, descriptions, labeled requirement sections, and general-education tags.\n\\
         parse-sequence-inline-segments splits a texts array into ordered sequence alternatives.\n\\
         parse-sequence-course-codes expands carried course numbers using supplied explicit-code and subject context.\n\
         parse-grc-annual-schedule-candidates parses an {{html, baseUrl}} page into sorted annual schedule links.\n\
         filter-grc-annual-schedules keeps the current and most recent prior schedule year.\n\
         parse-grc-catalog-archive-entries parses an {{html, baseUrl}} object into sorted archive entries.\n\
         parse-grc-current-catalog-details parses an {{html, catalogRootUrl, expectedCatalogYearLabel}} object into current catalog details.\n\
         render-equivalency-guide-typescript emits the generated TypeScript module for officialRules and derivedRules arrays.\n\
         render-transfer-equivalency-catalog-typescript projects accepted, non-obsolete category rows into the lightweight catalog module.\n\
         synthesize-uw-grc-derived-rules emits derived rules from officialRules, optional structuredRules, and items containing a spec plus supportRuleSelectors.\n\
         build-equivalency-guide-report emits the deterministic report object and Markdown from parsed rows and rules.\n\
         collect-opportunities fetches only source URLs already listed in starter-opportunities.json and emits a review-only report; it never edits the catalog.\n\
         audit-planner-publish-readiness writes one of the four transfer-planner publish-readiness audits; --run-checks executes the release-readiness command sequence.\n\
         audit-source-backed-coverage scores the planner context JSON on stdin and writes the source-backed coverage, generated-registry, or mapping audit. The CJS launcher remains the npm entry point because it supplies the TypeScript planner runtime.\n\
         verify-source-pipeline-core reads a JSON context from stdin; sourceRoot loads existing reports, and loadCanonicalRegistry=true reads planner TypeScript registries through the read-only Node adapter. It emits report JSON and Markdown in a JSON envelope and never writes generated report files."
    );
}
