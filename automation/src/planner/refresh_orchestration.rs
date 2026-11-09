use super::{GrcPublicMaterials, refresh_deadlines, refresh_uw_catalog};
use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use url::Url;

#[path = "legacy_artifact_status.rs"]
mod legacy_artifact_status;
#[path = "source_year_coverage.rs"]
mod source_year_coverage;

const REFRESH_SECTIONS: &[SectionDefinition] = &[
    SectionDefinition {
        id: "grc-discovery",
        title: "Refresh: GRC discovery and baseline",
        description: "Discover Green River public materials, verify source-year coverage, and regenerate Green River associate tracks.",
        steps: &[
            StepDefinition {
                label: "Discover Green River public materials",
                condition: StepCondition::NotSkipDownloads,
            },
            StepDefinition {
                label: "Check source year coverage",
                condition: StepCondition::Always,
            },
            StepDefinition {
                label: "Generate Green River associate tracks",
                condition: StepCondition::NotSkipDownloads,
            },
        ],
    },
    SectionDefinition {
        id: "source-audit",
        title: "Refresh: source audit and gap detection",
        description: "Check official source links, discover primary sources, build the source queue, and classify hidden source gaps.",
        steps: &[
            StepDefinition {
                label: "Check official source links",
                condition: StepCondition::CheckSources,
            },
            StepDefinition {
                label: "Discover primary official sources",
                condition: StepCondition::NotSkipDownloads,
            },
            StepDefinition {
                label: "Build primary-source automation queue",
                condition: StepCondition::Always,
            },
            StepDefinition {
                label: "Promote high-confidence primary sources",
                condition: StepCondition::Always,
            },
            StepDefinition {
                label: "Classify hidden source gaps",
                condition: StepCondition::Always,
            },
        ],
    },
    SectionDefinition {
        id: "requirement-parsing",
        title: "Refresh: requirement parsing and fingerprint intelligence",
        description: "Parse UW major requirement sources, rebuild source and parsed-fact fingerprints, refresh parser-backed source gaps, classify source changes, and validate canonical source-pipeline invariants.",
        steps: &[
            StepDefinition {
                label: "Prepare source-audit validation prerequisites",
                condition: StepCondition::PrepareRequirementSection,
            },
            StepDefinition {
                label: "Parse UW major requirement sources",
                condition: StepCondition::ParseRequirements,
            },
            StepDefinition {
                label: "Build source fingerprints and classify changes",
                condition: StepCondition::Always,
            },
            StepDefinition {
                label: "Reclassify hidden source gaps after requirement parsing",
                condition: StepCondition::Always,
            },
            StepDefinition {
                label: "Validate source pipeline invariants",
                condition: StepCondition::Always,
            },
        ],
    },
    SectionDefinition {
        id: "schedule-cache",
        title: "Refresh: annual schedule cache",
        description: "Download or reuse the current Green River annual schedule PDFs used by the planner.",
        steps: &[StepDefinition {
            label: "Snapshot Green River annual schedules",
            condition: StepCondition::NotSkipDownloads,
        }],
    },
    SectionDefinition {
        id: "deadline-refresh",
        title: "Refresh: deadline sources",
        description: "Parse high-value deadline sources: Green River registrar dates, Green River financial aid deadlines, and UW Seattle transfer application deadlines.",
        steps: &[StepDefinition {
            label: "Refresh deadline sources",
            condition: StepCondition::NotSkipDownloads,
        }],
    },
    SectionDefinition {
        id: "catalog-and-generation",
        title: "Refresh: catalog ingest and generated outputs",
        description: "Generate the source bootstrap, parse equivalencies, ingest catalogs, merge metadata, rebuild availability, and refresh docs.",
        steps: &[
            StepDefinition {
                label: "Generate source bootstrap",
                condition: StepCondition::Always,
            },
            StepDefinition {
                label: "Parse UW Green River equivalency guide",
                condition: StepCondition::NotSkipDownloads,
            },
            StepDefinition {
                label: "Ingest Green River course catalog",
                condition: StepCondition::NotSkipDownloads,
            },
            StepDefinition {
                label: "Ingest UW course catalogs",
                condition: StepCondition::NotSkipDownloads,
            },
            StepDefinition {
                label: "Generate merged course metadata",
                condition: StepCondition::Always,
            },
            StepDefinition {
                label: "Generate Green River availability registry",
                condition: StepCondition::Always,
            },
            StepDefinition {
                label: "Generate student runtime planner bundle",
                condition: StepCondition::Always,
            },
            StepDefinition {
                label: "Generate planner docs",
                condition: StepCondition::Always,
            },
        ],
    },
    SectionDefinition {
        id: "verification",
        title: "Refresh: verification suite",
        description: "Run source-registry audits, the owner audit, TypeScript typecheck, and planner tests after the refresh finishes.",
        steps: &[
            StepDefinition {
                label: "Validate source pipeline invariants",
                condition: StepCondition::VerifyOnly,
            },
            StepDefinition {
                label: "Refresh requirement-diff classification report",
                condition: StepCondition::RunVerification,
            },
            StepDefinition {
                label: "Audit transfer planner owners",
                condition: StepCondition::RunVerification,
            },
            StepDefinition {
                label: "Run parser extraction and source-discovery tests",
                condition: StepCondition::RunVerification,
            },
            StepDefinition {
                label: "Audit runtime coverage (blocking)",
                condition: StepCondition::RunVerification,
            },
            StepDefinition {
                label: "Run closed-loop auto-repair pass",
                condition: StepCondition::ClosedLoopRepair,
            },
            StepDefinition {
                label: "Audit generated source registry",
                condition: StepCondition::RunVerification,
            },
            StepDefinition {
                label: "Audit UW-GRC mapping regressions",
                condition: StepCondition::RunVerification,
            },
            StepDefinition {
                label: "Run TypeScript typecheck",
                condition: StepCondition::RunVerification,
            },
            StepDefinition {
                label: "Check legacy planner service-test diagnostics",
                condition: StepCondition::RunVerification,
            },
        ],
    },
];

const TS_NODE_COMPILER_OPTIONS: &str = r#"{"module":"Node16","moduleResolution":"node16"}"#;
const TS_NODE_TEST_COMPILER_OPTIONS: &str = r#"{"module":"CommonJS","moduleResolution":"node","jsx":"react-jsx","baseUrl":".","paths":{"@/*":["./*"]}}"#;

#[derive(Clone, Copy)]
struct SectionDefinition {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    steps: &'static [StepDefinition],
}

#[derive(Clone, Copy)]
struct StepDefinition {
    label: &'static str,
    condition: StepCondition,
}

#[derive(Clone, Copy)]
enum StepCondition {
    Always,
    NotSkipDownloads,
    CheckSources,
    PrepareRequirementSection,
    ParseRequirements,
    VerifyOnly,
    RunVerification,
    ClosedLoopRepair,
}

#[derive(Clone, Debug, Default)]
struct RefreshOptions {
    verify_only: bool,
    skip_source_check: bool,
    skip_requirement_parse: bool,
    skip_downloads: bool,
    skip_verify: bool,
    skip_auto_repair: bool,
    force_refresh_downloads: bool,
    only_section: Option<String>,
    start_section: Option<String>,
    target_plan_id: Option<String>,
}

#[derive(Clone, Debug)]
struct PlannedSection {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    steps: Vec<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StepPlanJson {
    mode: &'static str,
    count: usize,
    labels: Vec<String>,
    selected_section_ids: Vec<String>,
    sections: Vec<StepPlanSectionJson>,
    available_sections: Vec<AvailableSectionJson>,
    flags: StepPlanFlagsJson,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StepPlanSectionJson {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    count: usize,
    labels: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AvailableSectionJson {
    id: &'static str,
    title: &'static str,
    description: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StepPlanFlagsJson {
    verify_only: bool,
    skip_source_check: bool,
    skip_requirement_parse: bool,
    skip_downloads: bool,
    skip_verify: bool,
    skip_auto_repair: bool,
    force_refresh_downloads: bool,
    only_section: Option<String>,
    start_section: Option<String>,
    target_plan_id: Option<String>,
}

#[derive(Default)]
struct DownloadStats {
    downloaded: usize,
    reused: usize,
    fallback_used: usize,
}

#[derive(Default)]
struct RefreshSummary {
    steps: Vec<String>,
    skipped_steps: Vec<SkippedStep>,
    downloads: DownloadStats,
}

struct SkippedStep {
    label: String,
}

struct AutoRepairPlan {
    registry_repair: Option<Value>,
    promoted_repair: Option<Value>,
    fingerprint_repair: Option<Value>,
    should_run_parser: bool,
    should_run_fingerprint_refresh: bool,
    target_plan_ids: Vec<String>,
}

pub fn run_planner_refresh(repository_root: &Path, args: &[String]) -> Result<(), String> {
    ensure_planner_tmp_layout(&repository_root.join("source"))?;
    if has_arg(args, "--help") || has_arg(args, "-h") {
        print_help();
        return Ok(());
    }

    let options = build_options(args)?;
    let section_plan = build_section_plan(&options)?;
    let planned_step_labels = section_plan
        .iter()
        .flat_map(|section| section.steps.iter().map(|label| (*label).to_string()))
        .collect::<Vec<_>>();

    if has_arg(args, "--print-step-plan-json") {
        let plan = build_step_plan_json(&options, &section_plan, &planned_step_labels);
        let encoded = serde_json::to_string(&plan)
            .map_err(|error| format!("could not encode planner refresh step plan: {error}"))?;
        println!("{encoded}");
        return Ok(());
    }
    if planned_step_labels.is_empty() {
        return Err("The selected refresh section plan has no runnable steps. Adjust the section/skip flags and try again.".to_string());
    }

    let mut executor =
        RefreshExecutor::new(repository_root, options, section_plan, planned_step_labels);
    executor.run()
}

fn build_options(args: &[String]) -> Result<RefreshOptions, String> {
    let options = RefreshOptions {
        verify_only: has_arg(args, "--verify-only"),
        skip_source_check: has_arg(args, "--skip-source-check"),
        skip_requirement_parse: has_arg(args, "--skip-requirement-parse"),
        skip_downloads: has_arg(args, "--skip-downloads"),
        skip_verify: has_arg(args, "--skip-verify"),
        skip_auto_repair: has_arg(args, "--skip-auto-repair"),
        force_refresh_downloads: has_arg(args, "--refresh-downloads"),
        only_section: get_arg_value(args, "--only-section"),
        start_section: get_arg_value(args, "--start-section"),
        target_plan_id: get_arg_value(args, "--target-plan-id"),
    };
    if options.only_section.is_some() && options.start_section.is_some() {
        return Err("Use either --only-section or --start-section, not both.".to_string());
    }
    Ok(options)
}

fn build_section_plan(options: &RefreshOptions) -> Result<Vec<PlannedSection>, String> {
    let selected = if options.verify_only {
        vec![section_by_id("verification").expect("verification section is registered")]
    } else if let Some(section_id) = options.only_section.as_deref() {
        let Some(section) = section_by_id(section_id) else {
            return Err(format!(
                "Unknown refresh section \"{section_id}\". Valid sections: {}",
                REFRESH_SECTIONS
                    .iter()
                    .map(|section| section.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        vec![section]
    } else if let Some(section_id) = options.start_section.as_deref() {
        let Some(start_index) = REFRESH_SECTIONS
            .iter()
            .position(|section| section.id == section_id)
        else {
            return Err(format!(
                "Unknown refresh start section \"{section_id}\". Valid sections: {}",
                REFRESH_SECTIONS
                    .iter()
                    .map(|section| section.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        REFRESH_SECTIONS[start_index..].to_vec()
    } else {
        REFRESH_SECTIONS.to_vec()
    };

    Ok(selected
        .into_iter()
        .filter_map(|section| {
            let steps = section
                .steps
                .iter()
                .filter(|step| condition_matches(step.condition, options))
                .map(|step| step.label)
                .collect::<Vec<_>>();
            (!steps.is_empty()).then_some(PlannedSection {
                id: section.id,
                title: section.title,
                description: section.description,
                steps,
            })
        })
        .collect())
}

fn condition_matches(condition: StepCondition, options: &RefreshOptions) -> bool {
    match condition {
        StepCondition::Always => true,
        StepCondition::NotSkipDownloads => !options.skip_downloads,
        StepCondition::CheckSources => !options.skip_source_check && !options.skip_downloads,
        StepCondition::PrepareRequirementSection => {
            options.only_section.as_deref() == Some("requirement-parsing")
                || options.start_section.as_deref() == Some("requirement-parsing")
        }
        StepCondition::ParseRequirements => {
            !options.skip_requirement_parse && !options.skip_downloads
        }
        StepCondition::VerifyOnly => options.verify_only,
        StepCondition::RunVerification => !options.skip_verify || options.verify_only,
        StepCondition::ClosedLoopRepair => {
            !options.verify_only
                && !options.skip_verify
                && !options.skip_auto_repair
                && !options.skip_downloads
        }
    }
}

fn section_by_id(id: &str) -> Option<SectionDefinition> {
    REFRESH_SECTIONS
        .iter()
        .copied()
        .find(|section| section.id == id)
}

fn build_step_plan_json(
    options: &RefreshOptions,
    sections: &[PlannedSection],
    labels: &[String],
) -> StepPlanJson {
    StepPlanJson {
        mode: if options.verify_only {
            "verify-only"
        } else {
            "full-refresh"
        },
        count: labels.len(),
        labels: labels.to_vec(),
        selected_section_ids: sections
            .iter()
            .map(|section| section.id.to_string())
            .collect(),
        sections: sections
            .iter()
            .map(|section| StepPlanSectionJson {
                id: section.id,
                title: section.title,
                description: section.description,
                count: section.steps.len(),
                labels: section
                    .steps
                    .iter()
                    .map(|label| (*label).to_string())
                    .collect(),
            })
            .collect(),
        available_sections: REFRESH_SECTIONS
            .iter()
            .map(|section| AvailableSectionJson {
                id: section.id,
                title: section.title,
                description: section.description,
            })
            .collect(),
        flags: StepPlanFlagsJson {
            verify_only: options.verify_only,
            skip_source_check: options.skip_source_check,
            skip_requirement_parse: options.skip_requirement_parse,
            skip_downloads: options.skip_downloads,
            skip_verify: options.skip_verify,
            skip_auto_repair: options.skip_auto_repair,
            force_refresh_downloads: options.force_refresh_downloads,
            only_section: options.only_section.clone(),
            start_section: options.start_section.clone(),
            target_plan_id: options.target_plan_id.clone(),
        },
    }
}

fn print_help() {
    println!(
        "Usage: node scripts/planner/refresh-transfer-planner-sources.cjs [options]\n\nOptions:\n  --verify-only              Run verification without refreshing sources.\n  --only-section <id>        Run one refresh section.\n  --start-section <id>       Start at one section and run the remainder.\n  --target-plan-id <id>      Limit supported steps to one planner plan.\n  --skip-downloads           Reuse cached source downloads.\n  --skip-source-check        Skip official source-link checks.\n  --skip-requirement-parse   Skip requirement parsing.\n  --skip-verify              Skip the verification suite.\n  --skip-auto-repair         Skip automatic repair planning.\n  --refresh-downloads        Force schedule downloads to refresh.\n  --print-step-plan-json     Print the selected step plan and exit.\n  --help, -h                 Show this help and exit.\n\nSections: {}",
        REFRESH_SECTIONS
            .iter()
            .map(|section| section.id)
            .collect::<Vec<_>>()
            .join(", ")
    );
}

fn has_arg(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn get_arg_value(args: &[String], flag: &str) -> Option<String> {
    let direct_prefix = format!("{flag}=");
    if let Some(direct_match) = args.iter().find(|arg| arg.starts_with(&direct_prefix)) {
        let value = trim_js_whitespace(&direct_match[direct_prefix.len()..]);
        return (!value.is_empty()).then(|| value.to_string());
    }
    let Some(index) = args.iter().position(|arg| arg == flag) else {
        return None;
    };
    let Some(value) = args.get(index + 1).filter(|value| !value.starts_with("--")) else {
        return None;
    };
    let value = trim_js_whitespace(value);
    (!value.is_empty()).then(|| value.to_string())
}

fn node_options_with_heap_limit(existing: Option<&str>) -> String {
    let mut options = existing
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string();
    if !options.is_empty() {
        options.push(' ');
    }
    options.push_str("--max-old-space-size=8192");
    options
}

#[derive(Clone, Copy)]
enum SectionMode {
    VerifyOnly,
    Full,
    TargetedVerify,
    TargetedRefresh,
}

impl SectionMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::VerifyOnly => "verify-only",
            Self::Full => "full-refresh",
            Self::TargetedVerify => "targeted-verify-only",
            Self::TargetedRefresh => "targeted-refresh",
        }
    }
}

struct RefreshExecutor<'a> {
    repository_root: &'a Path,
    source_root: PathBuf,
    options: RefreshOptions,
    section_plan: Vec<PlannedSection>,
    planned_step_labels: Vec<String>,
    next_step_index: usize,
    summary: RefreshSummary,
    grc_public_materials: Option<GrcPublicMaterials>,
    download_client: Option<Client>,
    download_next_allowed: HashMap<String, Instant>,
    started_at: Instant,
    finished_at: Option<Instant>,
}

impl<'a> RefreshExecutor<'a> {
    fn new(
        repository_root: &'a Path,
        options: RefreshOptions,
        section_plan: Vec<PlannedSection>,
        planned_step_labels: Vec<String>,
    ) -> Self {
        Self {
            repository_root,
            source_root: repository_root.join("source"),
            options,
            section_plan,
            planned_step_labels,
            next_step_index: 0,
            summary: RefreshSummary::default(),
            grc_public_materials: None,
            download_client: None,
            download_next_allowed: HashMap::new(),
            started_at: Instant::now(),
            finished_at: None,
        }
    }

    fn run(&mut self) -> Result<(), String> {
        self.print_target_fallback_messages();
        println!("Starting transfer planner refresh pipeline...");
        println!("Planned tracked steps: {}.", self.planned_step_labels.len());
        println!(
            "Selected refresh sections: {}.",
            self.section_plan
                .iter()
                .map(|section| section.id)
                .collect::<Vec<_>>()
                .join(", ")
        );
        if let Some(target_plan_id) = &self.options.target_plan_id {
            println!("Selected target plan: {target_plan_id}.");
        }

        let sections = self
            .section_plan
            .iter()
            .map(|section| section.id)
            .collect::<Vec<_>>();
        for section_id in sections {
            self.run_selected_section(section_id)?;
        }
        self.finished_at = Some(Instant::now());
        self.print_refresh_summary();
        println!("Transfer planner refresh pipeline complete.");
        Ok(())
    }

    fn print_target_fallback_messages(&self) {
        let Some(target_plan_id) = self.options.target_plan_id.as_deref() else {
            return;
        };
        let can_run_discovery = self
            .source_artifact_path("transfer-planner-primary-source-discovery.json")
            .exists();
        let can_run_parse = self
            .source_artifact_path("transfer-planner-requirement-source-parse-report.json")
            .exists();
        if !can_run_discovery {
            if self.options.skip_downloads {
                println!(
                    "Targeted discovery requested for {target_plan_id}, but no existing discovery baseline was found. No-download mode will require the cached full discovery report."
                );
            } else {
                println!(
                    "Targeted discovery requested for {target_plan_id}, but no existing discovery baseline was found. Running full discovery for safety."
                );
            }
        }
        if !can_run_parse {
            if self.options.skip_downloads {
                println!(
                    "Targeted requirement parsing requested for {target_plan_id}, but no existing parse baseline was found. No-download mode will require the cached full parse report."
                );
            } else {
                println!(
                    "Targeted requirement parsing requested for {target_plan_id}, but no existing parse baseline was found. Running full parsing for safety."
                );
            }
        }
    }

    fn tracked_step<T>(
        &mut self,
        label: &str,
        action: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        self.next_step_index += 1;
        let tracked_label = format!(
            "[{}/{}] {label}",
            self.next_step_index,
            self.planned_step_labels.len()
        );
        println!();
        println!("== {tracked_label} ==");
        let started = Instant::now();
        let result = action(self)?;
        println!("Completed in {}.", format_duration(started.elapsed()));
        self.summary.steps.push(label.to_string());
        Ok(result)
    }

    fn skip_step(&mut self, label: &str, reason: &str) {
        self.summary.skipped_steps.push(SkippedStep {
            label: label.to_string(),
        });
        println!("Skipping {label} ({reason}).");
    }

    fn run_selected_section(&mut self, section_id: &str) -> Result<(), String> {
        match section_id {
            "grc-discovery" => self.run_grc_discovery_section(),
            "source-audit" => self.run_source_audit_section(),
            "requirement-parsing" => self.run_requirement_parsing_section(),
            "schedule-cache" => self.run_schedule_cache_section(),
            "deadline-refresh" => self.run_deadline_refresh_section(),
            "catalog-and-generation" => self.run_catalog_generation_section(),
            "verification" => self.run_verification_section(),
            other => Err(format!("Unsupported refresh section \"{other}\".")),
        }
    }

    fn run_grc_discovery_section(&mut self) -> Result<(), String> {
        let materials = if !self.options.skip_downloads {
            self.tracked_step("Discover Green River public materials", |this| {
                this.get_grc_public_materials()
            })?
        } else {
            self.get_grc_public_materials()?
        };
        let source_root = self.source_root.clone();
        self.tracked_step("Check source year coverage", move |_| {
            source_year_coverage::write_report(&source_root, &materials)
        })?;
        if !self.options.skip_downloads {
            self.tracked_step("Generate Green River associate tracks", |this| {
                this.run_node_script(
                    "scripts/planner/generate-transfer-planner-grc-associate-tracks.cjs",
                    &[],
                )
            })?;
        } else {
            self.reuse_cached_artifact(
                "Green River associate-track generated output",
                &self
                    .source_root
                    .join("constants/transfer-planner-source/grc-associate-tracks.generated.ts"),
            )?;
        }
        Ok(())
    }

    fn run_source_audit_section(&mut self) -> Result<(), String> {
        if !self.options.skip_source_check && !self.options.skip_downloads {
            self.tracked_step("Check official source links", |this| {
                this.run_node_script("scripts/planner/check-transfer-planner-sources.cjs", &[])
            })?;
        } else if self.options.skip_source_check {
            self.skip_step("Check official source links", "--skip-source-check");
        } else {
            self.skip_step("Check official source links", "--skip-downloads");
        }

        if !self.options.skip_downloads {
            let args = self.build_target_plan_args(self.can_run_targeted_discovery());
            self.tracked_step("Discover primary official sources", |this| {
                this.run_node_script(
                    "scripts/planner/discover-transfer-planner-primary-sources.cjs",
                    &args,
                )
            })?;
        } else {
            self.reuse_cached_artifact(
                "primary official-source discovery report",
                &self.source_artifact_path("transfer-planner-primary-source-discovery.json"),
            )?;
        }
        self.tracked_step("Build primary-source automation queue", |this| {
            this.run_large_node(
                "scripts/planner/build-transfer-planner-primary-source-review-queue.cjs",
                &[],
            )
        })?;
        self.tracked_step("Promote high-confidence primary sources", |this| {
            this.run_large_node(
                "scripts/planner/build-transfer-planner-primary-source-promotions.cjs",
                &[],
            )
        })?;
        let args = self.build_target_plan_args(true);
        self.tracked_step("Classify hidden source gaps", |this| {
            this.run_large_node(
                "scripts/planner/build-transfer-planner-source-gap-report.cjs",
                &args,
            )
        })?;
        Ok(())
    }

    fn run_requirement_parsing_section(&mut self) -> Result<(), String> {
        if self.options.only_section.as_deref() == Some("requirement-parsing")
            || self.options.start_section.as_deref() == Some("requirement-parsing")
        {
            self.tracked_step("Prepare source-audit validation prerequisites", |this| {
                this.prepare_source_audit_validation_prerequisites()
            })?;
        }
        if !self.options.skip_requirement_parse && !self.options.skip_downloads {
            let args = self.build_target_plan_args(self.can_run_targeted_requirement_parse());
            self.tracked_step("Parse UW major requirement sources", |this| {
                this.run_large_node(
                    "scripts/planner/parse-transfer-planner-requirement-sources.cjs",
                    &args,
                )
            })?;
        } else if self.options.skip_requirement_parse {
            self.skip_step(
                "Parse UW major requirement sources",
                "--skip-requirement-parse",
            );
        } else {
            self.reuse_cached_artifact(
                "requirement-source parse report",
                &self.source_artifact_path("transfer-planner-requirement-source-parse-report.json"),
            )?;
        }

        self.tracked_step("Build source fingerprints and classify changes", |this| {
            this.run_node_script(
                "scripts/planner/build-transfer-planner-source-fingerprints.cjs",
                &[],
            )
        })?;
        let args = self.build_target_plan_args(true);
        self.tracked_step(
            "Reclassify hidden source gaps after requirement parsing",
            |this| {
                this.run_large_node(
                    "scripts/planner/build-transfer-planner-source-gap-report.cjs",
                    &args,
                )
            },
        )?;
        self.tracked_step("Validate source pipeline invariants", |this| {
            this.run_source_pipeline_validation_with_auto_parse_repair()
        })?;
        Ok(())
    }

    fn run_schedule_cache_section(&mut self) -> Result<(), String> {
        if !self.options.skip_downloads {
            let materials = self.get_grc_public_materials()?;
            let schedules = materials
                .annual_schedules
                .iter()
                .map(|entry| {
                    (
                        format!("Green River annual schedule {}", entry.label),
                        entry.url.clone(),
                        PathBuf::from(&entry.output_path),
                    )
                })
                .collect::<Vec<_>>();
            let download_stats = self
                .tracked_step("Snapshot Green River annual schedules", |this| {
                    this.refresh_schedule_downloads(&schedules)
                })?;
            self.summary.downloads = download_stats;
        } else {
            self.skip_step("Snapshot Green River annual schedules", "--skip-downloads");
        }
        Ok(())
    }

    fn run_deadline_refresh_section(&mut self) -> Result<(), String> {
        if !self.options.skip_downloads {
            self.tracked_step("Refresh deadline sources", |this| {
                this.refresh_deadlines_direct()
            })?;
        }
        Ok(())
    }

    fn run_catalog_generation_section(&mut self) -> Result<(), String> {
        self.tracked_step("Generate source bootstrap", |this| {
            this.run_node_script(
                "scripts/planner/generate-transfer-planner-source-bootstrap.cjs",
                &[],
            )
        })?;
        if !self.options.skip_downloads {
            self.tracked_step("Parse UW Green River equivalency guide", |this| {
                this.run_command(
                    "cargo",
                    &[
                        "run".to_string(),
                        "--locked".to_string(),
                        "--manifest-path".to_string(),
                        "../automation/equivalency-refresh/Cargo.toml".to_string(),
                        "--".to_string(),
                        "refresh-equivalencies".to_string(),
                    ],
                )
            })?;
            self.tracked_step("Ingest Green River course catalog", |this| {
                this.run_node_script("scripts/planner/ingest-grc-catalog.cjs", &[])
            })?;
            self.tracked_step("Ingest UW course catalogs", |this| {
                this.ingest_uw_catalog_direct(false)
            })?;
        } else {
            self.reuse_cached_artifact(
                "UW-GRC equivalency guide parse report",
                &self.source_artifact_path("transfer-planner-equivalency-guide-parse.json"),
            )?;
            self.reuse_cached_artifact(
                "Green River catalog ingest report",
                &self.source_artifact_path("transfer-planner-grc-catalog-ingest.json"),
            )?;
            self.reuse_cached_artifact(
                "UW catalog ingest report",
                &self.source_artifact_path("transfer-planner-uw-catalog-ingest.json"),
            )?;
        }
        self.tracked_step("Generate merged course metadata", |this| {
            this.run_node_script(
                "scripts/planner/generate-transfer-planner-course-metadata.cjs",
                &[],
            )
        })?;
        self.tracked_step("Generate Green River availability registry", |this| {
            this.run_node_script(
                "scripts/planner/generate-transfer-planner-grc-availability.cjs",
                &[],
            )
        })?;
        self.tracked_step("Generate student runtime planner bundle", |this| {
            this.run_node_script(
                "scripts/planner/generate-transfer-planner-student-runtime.cjs",
                &[],
            )
        })?;
        self.tracked_step("Generate planner docs", |this| {
            this.run_ts_node("scripts/planner/generate-transfer-planner-docs.ts")
        })?;
        Ok(())
    }

    fn run_verification_section(&mut self) -> Result<(), String> {
        if !self.options.skip_verify || self.options.verify_only {
            self.run_verification()?;
        } else {
            self.skip_step("Refresh verification suite", "--skip-verify");
        }
        Ok(())
    }

    fn run_verification(&mut self) -> Result<(), String> {
        if self.options.verify_only {
            self.tracked_step("Validate source pipeline invariants", |this| {
                this.run_source_pipeline_validation_with_auto_parse_repair()
            })?;
        }
        let target_args = self.build_target_plan_args(true);
        self.tracked_step("Refresh requirement-diff classification report", |this| {
            this.run_node_script(
                "scripts/planner/build-transfer-planner-requirement-diff-report.cjs",
                &target_args,
            )
        })?;
        self.tracked_step("Audit transfer planner owners", |this| {
            let result = this.run_node_script(
                "scripts/planner/verify-transfer-planner-owner-audit.cjs",
                &target_args,
            );
            if let Err(error) = result {
                this.run_closed_loop_auto_repair_before_rethrow(&error)?;
                return Err(error);
            }
            Ok(())
        })?;
        self.tracked_step("Run parser extraction and source-discovery tests", |this| {
            this.run_package_script("planner:test:parser")
        })?;
        self.tracked_step("Audit runtime coverage (blocking)", |this| {
            let result = this.run_node_script(
                "scripts/planner/audit-transfer-planner-source-backed-coverage.cjs",
                &target_args,
            );
            if let Err(error) = result {
                this.run_closed_loop_auto_repair_before_rethrow(&error)?;
                return Err(error);
            }
            Ok(())
        })?;
        if !self.options.skip_auto_repair
            && !self.options.skip_downloads
            && !self.options.verify_only
        {
            self.tracked_step("Run closed-loop auto-repair pass", |this| {
                this.run_closed_loop_auto_repair()
            })?;
        }
        let mut generated_args = vec!["--generated-registry-only".to_string()];
        generated_args.extend(target_args.clone());
        self.tracked_step("Audit generated source registry", |this| {
            this.run_node_script(
                "scripts/planner/audit-transfer-planner-source-backed-coverage.cjs",
                &generated_args,
            )
        })?;
        let mut mapping_args = vec!["--mapping-only".to_string()];
        mapping_args.extend(target_args);
        self.tracked_step("Audit UW-GRC mapping regressions", |this| {
            this.run_node_script(
                "scripts/planner/audit-transfer-planner-source-backed-coverage.cjs",
                &mapping_args,
            )
        })?;
        self.tracked_step("Run TypeScript typecheck", |this| {
            this.run_npx(&["tsc", "--noEmit"])
        })?;
        self.tracked_step("Check legacy planner service-test diagnostics", |this| {
            this.refresh_legacy_service_test_status()
        })?;
        Ok(())
    }

    fn prepare_source_audit_validation_prerequisites(&mut self) -> Result<(), String> {
        let discovery = self.source_artifact_path("transfer-planner-primary-source-discovery.json");
        if !discovery.exists() {
            if self.options.skip_downloads {
                self.reuse_cached_artifact("primary official-source discovery report", &discovery)?;
            } else {
                self.run_node_script(
                    "scripts/planner/discover-transfer-planner-primary-sources.cjs",
                    &[],
                )?;
            }
        }
        let review_queue =
            self.source_artifact_path("transfer-planner-primary-source-review-queue.json");
        if !review_queue.exists() {
            self.run_large_node(
                "scripts/planner/build-transfer-planner-primary-source-review-queue.cjs",
                &[],
            )?;
        }
        let promotions =
            self.source_artifact_path("transfer-planner-primary-source-promotions.json");
        if !promotions.exists() {
            self.run_large_node(
                "scripts/planner/build-transfer-planner-primary-source-promotions.cjs",
                &[],
            )?;
        }
        Ok(())
    }

    fn run_source_pipeline_validation_with_auto_parse_repair(&mut self) -> Result<(), String> {
        const MAX_REPAIR_PASSES: usize = 3;
        let target_args = self.build_target_plan_args(true);
        for repair_pass in 0..=MAX_REPAIR_PASSES {
            match self.run_node_script(
                "scripts/planner/verify-transfer-planner-source-pipeline.cjs",
                &target_args,
            ) {
                Ok(()) => return Ok(()),
                Err(original_error) => {
                    if repair_pass >= MAX_REPAIR_PASSES || self.options.skip_auto_repair {
                        return Err(original_error);
                    }
                    let Some(plan) = self.get_source_pipeline_auto_repair_plan()? else {
                        return Err(original_error);
                    };
                    if !plan.should_run_parser && !plan.should_run_fingerprint_refresh {
                        return Err(original_error);
                    }
                    if let Some(repair) = &plan.registry_repair {
                        if js_truthy(repair.get("needed")) {
                            println!(
                                "Source-pipeline parser coverage drift detected: {} current and {} generated owner/source pair(s) need parser coverage.",
                                array_len(repair.get("missingCurrentSourceCoverage")),
                                array_len(repair.get("missingGeneratedSourceCoverage")),
                            );
                        }
                    }
                    if let Some(repair) = &plan.promoted_repair {
                        if js_truthy(repair.get("needed")) {
                            println!(
                                "Source-pipeline promoted-owner coverage drift detected: {} parsed block(s) and {} fingerprint(s) need refresh.",
                                array_len(repair.get("missingParsedOwnerIds")),
                                array_len(repair.get("missingFingerprintOwnerIds")),
                            );
                        }
                    }
                    if let Some(repair) = &plan.fingerprint_repair {
                        if js_truthy(repair.get("needed")) {
                            println!(
                                "Source-pipeline fingerprint drift detected: {} parsed-only owner(s), {} fingerprint-only owner(s).",
                                array_len(repair.get("parsedOnlyOwnerIds")),
                                array_len(repair.get("fingerprintOnlyOwnerIds")),
                            );
                        }
                    }
                    if plan.should_run_parser {
                        if self.options.skip_downloads {
                            return Err("Source-pipeline repair needs a requirement-source parser refresh, but --skip-downloads is running in cached-source mode. Run the normal planner refresh when online source refreshes are allowed.".to_string());
                        }
                        println!(
                            "Targeted parser repair plan(s): {}",
                            if plan.target_plan_ids.is_empty() {
                                "full parse".to_string()
                            } else {
                                plan.target_plan_ids.join(", ")
                            }
                        );
                        self.run_requirement_parser_repair_for_plans(&plan.target_plan_ids)?;
                    }
                    if plan.should_run_fingerprint_refresh {
                        self.run_node_script(
                            "scripts/planner/build-transfer-planner-source-fingerprints.cjs",
                            &[],
                        )?;
                    }
                    if plan.should_run_parser || plan.should_run_fingerprint_refresh {
                        self.regenerate_source_pipeline_runtime_outputs()?;
                    }
                    println!(
                        "Source-pipeline auto-repair pass {}/{} complete; re-running validation.",
                        repair_pass + 1,
                        MAX_REPAIR_PASSES
                    );
                }
            }
        }
        Err("source pipeline validation failed after auto-repair passes".to_string())
    }

    fn get_source_pipeline_auto_repair_plan(&self) -> Result<Option<AutoRepairPlan>, String> {
        let path = self.source_artifact_path("transfer-planner-source-pipeline-validation.json");
        let Some(report) = read_json_if_exists(&path)? else {
            return Ok(None);
        };
        let auto_repair = report.get("autoRepair");
        let registry_repair = auto_repair
            .and_then(|value| value.get("registryParserAlignment"))
            .filter(|value| !value.is_null())
            .cloned();
        let promoted_repair = auto_repair
            .and_then(|value| value.get("promotedOwnerCoverage"))
            .filter(|value| !value.is_null())
            .cloned();
        let fingerprint_repair = auto_repair
            .and_then(|value| value.get("fingerprintAlignment"))
            .filter(|value| !value.is_null())
            .cloned();
        let registry_needed = registry_repair
            .as_ref()
            .is_some_and(|repair| js_truthy(repair.get("needed")));
        let promoted_needed = promoted_repair
            .as_ref()
            .is_some_and(|repair| js_truthy(repair.get("needed")));
        let fingerprint_needed = fingerprint_repair
            .as_ref()
            .is_some_and(|repair| js_truthy(repair.get("needed")));
        let mut should_run_parser = false;
        let mut should_run_fingerprint_refresh = false;
        let mut target_plan_ids = Vec::new();

        if registry_needed {
            should_run_parser = true;
            should_run_fingerprint_refresh = true;
            append_plan_ids(
                &mut target_plan_ids,
                registry_repair
                    .as_ref()
                    .and_then(|value| value.get("targetPlanIds")),
            );
        }
        if promoted_needed {
            if array_len(
                promoted_repair
                    .as_ref()
                    .and_then(|value| value.get("missingParsedOwnerIds")),
            ) > 0
            {
                should_run_parser = true;
                append_plan_ids(
                    &mut target_plan_ids,
                    promoted_repair
                        .as_ref()
                        .and_then(|value| value.get("targetPlanIds")),
                );
            }
            if array_len(
                promoted_repair
                    .as_ref()
                    .and_then(|value| value.get("missingFingerprintOwnerIds")),
            ) > 0
            {
                should_run_fingerprint_refresh = true;
            }
        }
        if fingerprint_needed {
            should_run_fingerprint_refresh = true;
        }
        if !should_run_parser && !should_run_fingerprint_refresh {
            return Ok(None);
        }
        target_plan_ids.sort_by(|left, right| js_locale_compare(left, right));
        target_plan_ids.dedup();
        Ok(Some(AutoRepairPlan {
            registry_repair,
            promoted_repair,
            fingerprint_repair,
            should_run_parser,
            should_run_fingerprint_refresh,
            target_plan_ids,
        }))
    }

    fn run_requirement_parser_repair_for_plans(
        &mut self,
        target_plan_ids: &[String],
    ) -> Result<(), String> {
        let parse_report =
            self.source_artifact_path("transfer-planner-requirement-source-parse-report.json");
        if !parse_report.exists() || target_plan_ids.is_empty() {
            println!(
                "Source-pipeline repair is running a full requirement parse because no parse baseline or target plan list was available."
            );
            return self.run_large_node(
                "scripts/planner/parse-transfer-planner-requirement-sources.cjs",
                &[],
            );
        }
        for plan_id in target_plan_ids {
            println!("Source-pipeline repair is refreshing requirement sources for {plan_id}.");
            let args = ["--target-plan-id".to_string(), plan_id.clone()];
            self.run_large_node(
                "scripts/planner/parse-transfer-planner-requirement-sources.cjs",
                &args,
            )?;
        }
        Ok(())
    }

    fn regenerate_source_pipeline_runtime_outputs(&mut self) -> Result<(), String> {
        println!(
            "Source-pipeline repair updated parser/source artifacts; regenerating source bootstrap and student runtime."
        );
        let args = self.build_target_plan_args(true);
        self.run_large_node(
            "scripts/planner/build-transfer-planner-source-gap-report.cjs",
            &args,
        )?;
        self.run_node_script(
            "scripts/planner/generate-transfer-planner-source-bootstrap.cjs",
            &[],
        )?;
        self.run_node_script(
            "scripts/planner/generate-transfer-planner-student-runtime.cjs",
            &[],
        )
    }

    fn run_closed_loop_auto_repair_before_rethrow(
        &mut self,
        original_error: &str,
    ) -> Result<(), String> {
        if self.options.skip_auto_repair || self.options.skip_downloads {
            return Err(original_error.to_string());
        }
        println!(
            "Verification found planner automation debt; running closed-loop auto-repair before failing."
        );
        if let Err(repair_error) = self.run_closed_loop_auto_repair() {
            println!("Closed-loop auto-repair pass also failed: {repair_error}.");
        }
        Err(original_error.to_string())
    }

    fn run_closed_loop_auto_repair(&mut self) -> Result<(), String> {
        let mut args = vec!["--repair".to_string()];
        args.extend(self.build_target_plan_args(true));
        self.run_large_node(
            "scripts/planner/build-transfer-planner-auto-repair-plan.cjs",
            &args,
        )
    }

    fn refresh_legacy_service_test_status(&self) -> Result<(), String> {
        let status = legacy_artifact_status::refresh(&self.source_root)?;
        println!(
            "Legacy service-test artifact status: {}; retired {} stale artifact(s).",
            status.status, status.retired_artifact_count
        );
        println!(
            "Legacy service-test status report: {}",
            status.markdown_path.display()
        );
        if std::env::var("GATORGUIDE_RUN_LEGACY_TRANSFER_PLANNER_SERVICE_TESTS").as_deref()
            == Ok("1")
        {
            if let Err(error) =
                self.run_ts_node_test("scripts/planner/transfer-planner.service.test.ts")
            {
                println!(
                    "Transfer planner service tests failed; continuing because the runtime, generated-registry, mapping, parser, and TypeScript gates above are the blocking planner maintenance checks."
                );
                println!("{error}");
            }
        } else {
            println!(
                "Skipping legacy transfer planner service tests in maintenance. The parser, runtime, generated-registry, mapping, and TypeScript gates above are the blocking planner accuracy checks. Set GATORGUIDE_RUN_LEGACY_TRANSFER_PLANNER_SERVICE_TESTS=1 to run the legacy compatibility suite as an opt-in diagnostic."
            );
        }
        Ok(())
    }

    fn refresh_deadlines_direct(&self) -> Result<(), String> {
        let report = refresh_deadlines(self.repository_root)?;
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

    fn ingest_uw_catalog_direct(&self, cache_only: bool) -> Result<(), String> {
        let report = refresh_uw_catalog(self.repository_root, cache_only)?;
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

    fn get_grc_public_materials(&mut self) -> Result<GrcPublicMaterials, String> {
        if let Some(materials) = &self.grc_public_materials {
            return Ok(materials.clone());
        }
        let cache_only = self.options.skip_downloads;
        let materials = super::discover_grc_public_materials(
            self.repository_root,
            !cache_only,
            true,
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
            self.source_artifact_path("transfer-planner-grc-public-materials.json")
                .display()
        );
        self.grc_public_materials = Some(materials.clone());
        Ok(materials)
    }

    fn refresh_schedule_downloads(
        &mut self,
        schedules: &[(String, String, PathBuf)],
    ) -> Result<DownloadStats, String> {
        let mut stats = DownloadStats::default();
        let tmp_root = self.source_root.join(".tmp");
        fs::create_dir_all(&tmp_root).map_err(|error| {
            format!(
                "could not create planner temp directory {}: {error}",
                tmp_root.display()
            )
        })?;
        for (label, url, output_path) in schedules {
            if !self.options.force_refresh_downloads && output_path.exists() {
                println!("Keeping existing {label}: {}", output_path.display());
                stats.reused += 1;
                continue;
            }
            println!("Downloading {label}...");
            match self.download_file_with_retry(url, output_path) {
                Ok(()) => {
                    println!("Saved {}", output_path.display());
                    stats.downloaded += 1;
                }
                Err(error) if output_path.exists() => {
                    println!(
                        "Download failed for {label}; keeping existing cached file {}.",
                        output_path.display()
                    );
                    println!("Download error: {error}");
                    stats.fallback_used += 1;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(stats)
    }

    fn download_file_with_retry(&mut self, url: &str, output_path: &Path) -> Result<(), String> {
        let mut last_error = None;
        for attempt in 1..=3 {
            match self.download_file(url, output_path) {
                Ok(()) => return Ok(()),
                Err(error) => {
                    last_error = Some(error);
                    if attempt < 3 {
                        thread::sleep(Duration::from_millis(800 * attempt as u64));
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| format!("Download failed for {url}.")))
    }

    fn download_file(&mut self, url: &str, output_path: &Path) -> Result<(), String> {
        let scope = rate_limit_scope(url);
        self.wait_for_download_scope(&scope);
        let bytes = self.fetch_schedule_bytes(url, &scope)?;
        let parent = output_path.parent().ok_or_else(|| {
            format!(
                "schedule output path has no parent: {}",
                output_path.display()
            )
        })?;
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "could not create schedule output directory {}: {error}",
                parent.display()
            )
        })?;
        fs::write(output_path, bytes).map_err(|error| {
            format!(
                "could not write schedule download {}: {error}",
                output_path.display()
            )
        })
    }

    fn wait_for_download_scope(&mut self, scope: &str) {
        if let Some(next_allowed) = self.download_next_allowed.get(scope).copied() {
            if let Some(delay) = next_allowed.checked_duration_since(Instant::now()) {
                thread::sleep(delay);
            }
        }
    }

    fn mark_download_scope(&mut self, scope: &str) {
        let min_delay_ms = if scope.starts_with("scope:") {
            1_500
        } else {
            900
        };
        self.download_next_allowed.insert(
            scope.to_string(),
            Instant::now() + Duration::from_millis(min_delay_ms),
        );
    }

    fn fetch_schedule_bytes(&mut self, url: &str, scope: &str) -> Result<Vec<u8>, String> {
        if self.download_client.is_none() {
            self.download_client = Some(
                Client::builder()
                    .redirect(Policy::limited(20))
                    .build()
                    .map_err(|error| {
                        format!("could not create schedule download client: {error}")
                    })?,
            );
        }
        let client = self
            .download_client
            .as_ref()
            .expect("client initialized")
            .clone();
        for attempt in 0..=1 {
            let response = client
                .get(url)
                .timeout(Duration::from_secs(30))
                .header("user-agent", "GatorGuideTransferPlannerRefresh/1.0")
                .send();
            let response = match response {
                Ok(response) => response,
                Err(error) if attempt == 0 && retryable_network_error(&error) => {
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }
                Err(error) => {
                    self.mark_download_scope(scope);
                    return Err(if error.is_timeout() {
                        format!("Download transfer planner source timed out for {url}.")
                    } else {
                        format!("Download transfer planner source failed for {url}.")
                    });
                }
            };
            let status = response.status();
            if status.is_success() {
                self.mark_download_scope(scope);
                return response
                    .bytes()
                    .map(|bytes| bytes.to_vec())
                    .map_err(|error| {
                        format!("Download transfer planner source failed for {url}: {error}")
                    });
            }
            let body = response.text().unwrap_or_default();
            if attempt == 0 && retryable_http_status(status.as_u16()) {
                thread::sleep(Duration::from_millis(500));
                continue;
            }
            let body = sanitize_fetch_body(&body);
            self.mark_download_scope(scope);
            return Err(format!(
                "Download transfer planner source failed for {url}: {} {}{}",
                status.as_u16(),
                status.canonical_reason().unwrap_or(""),
                if body.is_empty() {
                    String::new()
                } else {
                    format!(" {body}")
                }
            ));
        }
        Err(format!(
            "Download transfer planner source failed for {url}."
        ))
    }

    fn run_node_script(&self, script: &str, args: &[String]) -> Result<(), String> {
        let mut command_args = Vec::with_capacity(args.len() + 1);
        command_args.push(script.to_string());
        command_args.extend(args.iter().cloned());
        self.run_command("node", &command_args)
    }

    fn run_large_node(&self, script: &str, args: &[String]) -> Result<(), String> {
        let mut command_args = Vec::with_capacity(args.len() + 2);
        command_args.push("--max-old-space-size=8192".to_string());
        command_args.push(script.to_string());
        command_args.extend(args.iter().cloned());
        self.run_command("node", &command_args)
    }

    fn run_package_script(&self, script: &str) -> Result<(), String> {
        self.run_command(npm_command_name(), &["run".to_string(), script.to_string()])
    }

    fn run_npx(&self, args: &[&str]) -> Result<(), String> {
        self.run_command(
            npx_command_name(),
            &args
                .iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>(),
        )
    }

    fn run_ts_node(&self, script: &str) -> Result<(), String> {
        let existing_node_options = std::env::var("NODE_OPTIONS").ok();
        let node_options = node_options_with_heap_limit(existing_node_options.as_deref());
        self.run_command_with_env(
            npx_command_name(),
            &[
                "ts-node".to_string(),
                "--project".to_string(),
                "tsconfig.json".to_string(),
                "--require".to_string(),
                "tsconfig-paths/register".to_string(),
                script.to_string(),
            ],
            &[
                ("NODE_OPTIONS", node_options),
                ("TS_NODE_TRANSPILE_ONLY", "true".to_string()),
                (
                    "TS_NODE_COMPILER_OPTIONS",
                    TS_NODE_COMPILER_OPTIONS.to_string(),
                ),
                ("TS_NODE_BASEURL", ".".to_string()),
            ],
        )
    }

    fn run_ts_node_test(&self, script: &str) -> Result<(), String> {
        self.run_command_with_env(
            "node",
            &[
                "--test".to_string(),
                "-r".to_string(),
                "ts-node/register".to_string(),
                "-r".to_string(),
                "tsconfig-paths/register".to_string(),
                script.to_string(),
            ],
            &[
                ("TS_NODE_TRANSPILE_ONLY", "true".to_string()),
                ("TS_NODE_SKIP_PROJECT", "true".to_string()),
                (
                    "TS_NODE_COMPILER_OPTIONS",
                    TS_NODE_TEST_COMPILER_OPTIONS.to_string(),
                ),
                ("TS_NODE_BASEURL", ".".to_string()),
            ],
        )
    }

    fn run_command(&self, program: &str, args: &[String]) -> Result<(), String> {
        self.run_command_with_env(program, args, &[])
    }

    fn run_command_with_env(
        &self,
        program: &str,
        args: &[String],
        environment: &[(&str, String)],
    ) -> Result<(), String> {
        let mut command = if cfg!(windows) && program.to_ascii_lowercase().ends_with(".cmd") {
            let mut cmd = Command::new("cmd");
            cmd.arg("/c").arg(program).args(args);
            cmd
        } else {
            let mut command = Command::new(program);
            command.args(args);
            command
        };
        command
            .current_dir(&self.source_root)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        if self.options.skip_downloads {
            command.env("GATORGUIDE_PLANNER_CACHE_ONLY", "1");
        }
        for (key, value) in environment {
            command.env(key, value);
        }
        let status = command
            .status()
            .map_err(|_| format!("Command failed: {program} {}", args.join(" ")))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("Command failed: {program} {}", args.join(" ")))
        }
    }

    fn build_target_plan_args(&self, use_target: bool) -> Vec<String> {
        if use_target {
            self.options
                .target_plan_id
                .clone()
                .into_iter()
                .flat_map(|target| ["--target-plan-id".to_string(), target])
                .collect()
        } else {
            Vec::new()
        }
    }

    fn can_run_targeted_discovery(&self) -> bool {
        self.options.target_plan_id.is_some()
            && self
                .source_artifact_path("transfer-planner-primary-source-discovery.json")
                .exists()
    }

    fn can_run_targeted_requirement_parse(&self) -> bool {
        self.options.target_plan_id.is_some()
            && self
                .source_artifact_path("transfer-planner-requirement-source-parse-report.json")
                .exists()
    }

    fn source_artifact_path(&self, filename: &str) -> PathBuf {
        self.source_root.join(".tmp").join("reports").join(filename)
    }

    fn reuse_cached_artifact(&self, label: &str, path: &Path) -> Result<(), String> {
        if !path.exists() {
            return Err(format!(
                "No-download mode needs cached {label} at {}, but it was not found. Run the normal planner refresh once to create it.",
                path.display()
            ));
        }
        println!("Using cached {label}: {}", path.display());
        Ok(())
    }

    fn print_refresh_summary(&self) {
        let executed = self
            .summary
            .steps
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let skipped = self
            .summary
            .skipped_steps
            .iter()
            .map(|step| step.label.as_str())
            .collect::<Vec<_>>();
        println!();
        println!("Refresh summary:");
        println!("- Mode: {}", self.refresh_mode().as_str());
        if let Some(target_plan_id) = &self.options.target_plan_id {
            println!("- Target plan: {target_plan_id}");
        }
        println!(
            "- Steps executed: {}; skipped: {}",
            self.summary.steps.len(),
            self.summary.skipped_steps.len()
        );
        println!(
            "- Schedule downloads: downloaded {}; reused {}; fallback-used {}",
            self.summary.downloads.downloaded,
            self.summary.downloads.reused,
            self.summary.downloads.fallback_used
        );
        let finished_at = self.finished_at.unwrap_or_else(Instant::now);
        println!(
            "- Duration: {}",
            format_duration(finished_at.duration_since(self.started_at))
        );
        println!(
            "  Executed steps (sample): {}",
            summarize_list(&executed, 8)
        );
        println!("  Skipped steps (sample): {}", summarize_list(&skipped, 8));
    }

    fn refresh_mode(&self) -> SectionMode {
        match (
            self.options.target_plan_id.is_some(),
            self.options.verify_only,
        ) {
            (true, true) => SectionMode::TargetedVerify,
            (true, false) => SectionMode::TargetedRefresh,
            (false, true) => SectionMode::VerifyOnly,
            (false, false) => SectionMode::Full,
        }
    }
}

fn ensure_planner_tmp_layout(source_root: &Path) -> Result<(), String> {
    let categories = [
        "reports",
        "snapshots",
        "error_logs",
        "logs",
        "exports",
        "cache",
        "downloads",
        "screenshots",
        "scratch",
        "qa",
        "builds",
    ];
    for category in categories {
        let path = source_root.join(".tmp").join(category);
        fs::create_dir_all(&path).map_err(|error| {
            format!(
                "could not create planner temp directory {}: {error}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn read_json_if_exists(path: &Path) -> Result<Option<Value>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("could not read JSON file {}: {error}", path.display()))?;
    serde_json::from_str(&content)
        .map(Some)
        .map_err(|error| format!("could not parse JSON file {}: {error}", path.display()))
}

fn append_plan_ids(targets: &mut Vec<String>, values: Option<&Value>) {
    let Some(values) = values.and_then(Value::as_array) else {
        return;
    };
    for value in values {
        if js_truthy(Some(value)) {
            let id = js_string(value);
            if !targets.contains(&id) {
                targets.push(id);
            }
        }
    }
}

fn array_len(value: Option<&Value>) -> usize {
    value.and_then(Value::as_array).map_or(0, Vec::len)
}

fn js_locale_compare(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| right.cmp(left))
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

fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
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

fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_millis().saturating_add(500) / 1_000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes}m {seconds}s")
}

fn summarize_list(values: &[&str], limit: usize) -> String {
    if values.is_empty() {
        return "none".to_string();
    }
    values
        .iter()
        .take(limit)
        .copied()
        .collect::<Vec<_>>()
        .join(", ")
}

fn retryable_http_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504)
}

fn retryable_network_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}

fn sanitize_fetch_body(body: &str) -> String {
    let raw = trim_js_whitespace(body);
    if raw.is_empty() {
        return String::new();
    }
    let lower = raw.to_ascii_lowercase();
    let has_html_tag = lower.match_indices("<html").any(|(index, _)| {
        raw.get(index + "<html".len()..)
            .and_then(|suffix| suffix.chars().next())
            .is_some_and(|character| character == '>' || is_js_whitespace(character))
    });
    if lower.contains("<!doctype html") || has_html_tag {
        return "[html-error-body]".to_string();
    }
    String::from_utf16_lossy(&raw.encode_utf16().take(500).collect::<Vec<_>>())
}

fn trim_js_whitespace(value: &str) -> &str {
    let start = value
        .char_indices()
        .find_map(|(index, character)| (!is_js_whitespace(character)).then_some(index));
    let Some(start) = start else {
        return "";
    };
    let end = value
        .char_indices()
        .rev()
        .find_map(|(index, character)| {
            (!is_js_whitespace(character)).then_some(index + character.len_utf8())
        })
        .unwrap_or(start);
    &value[start..end]
}

fn is_js_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'..='\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'..='\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

fn rate_limit_scope(url: &str) -> String {
    let Ok(parsed) = Url::parse(url) else {
        return "scope:unknown-host".to_string();
    };
    let mut host = parsed
        .host_str()
        .unwrap_or("unknown-host")
        .to_ascii_lowercase();
    if let Some(port) = parsed.port() {
        host.push(':');
        host.push_str(&port.to_string());
    }
    if host == "washington.edu" || host.ends_with(".washington.edu") {
        "scope:washington.edu".to_string()
    } else if host == "uw.edu"
        || host.ends_with(".uw.edu")
        || host == "uwb.edu"
        || host.ends_with(".uwb.edu")
    {
        "scope:uw.edu".to_string()
    } else if host == "greenriver.edu" || host.ends_with(".greenriver.edu") {
        "scope:greenriver.edu".to_string()
    } else {
        format!("host:{host}")
    }
}

fn npm_command_name() -> &'static str {
    if cfg!(windows) { "npm.cmd" } else { "npm" }
}

fn npx_command_name() -> &'static str {
    if cfg!(windows) { "npx.cmd" } else { "npx" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    fn test_directory() -> PathBuf {
        loop {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "gator-guide-refresh-exists-{}-{timestamp}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return path,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("could not create refresh test directory: {error}"),
            }
        }
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn refresh_flag_values_use_ecmascript_whitespace_trimming() {
        assert_eq!(
            get_arg_value(
                &args(&["--target-plan-id", "\u{FEFF}plan-id\u{FEFF}"]),
                "--target-plan-id"
            ),
            Some("plan-id".to_string())
        );
        assert_eq!(
            get_arg_value(
                &args(&["--target-plan-id=\u{0085}plan-id\u{0085}"]),
                "--target-plan-id"
            ),
            Some("\u{0085}plan-id\u{0085}".to_string())
        );
    }

    #[test]
    fn cached_artifact_presence_matches_node_exists_sync_for_directories() {
        let root = test_directory();
        let source_root = root.join("source");
        let discovery =
            source_root.join(".tmp/reports/transfer-planner-primary-source-discovery.json");
        fs::create_dir_all(&discovery).unwrap();
        let options = RefreshOptions {
            target_plan_id: Some("test-plan".to_string()),
            ..RefreshOptions::default()
        };
        let executor = RefreshExecutor::new(&root, options, Vec::new(), Vec::new());
        assert!(executor.can_run_targeted_discovery());
        assert!(!executor.can_run_targeted_requirement_parse());
        executor
            .reuse_cached_artifact("primary official-source discovery report", &discovery)
            .unwrap();
        assert!(read_json_if_exists(&discovery).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn default_refresh_plan_contains_the_ordered_pipeline_sections() {
        let options = build_options(&[]).unwrap();
        let plan = build_section_plan(&options).unwrap();
        assert_eq!(
            plan.iter().map(|section| section.id).collect::<Vec<_>>(),
            vec![
                "grc-discovery",
                "source-audit",
                "requirement-parsing",
                "schedule-cache",
                "deadline-refresh",
                "catalog-and-generation",
                "verification",
            ]
        );
        assert_eq!(plan.iter().flat_map(|section| &section.steps).count(), 31);
    }

    #[test]
    fn no_download_plan_excludes_live_source_work_but_keeps_cached_generators() {
        let options = build_options(&args(&["--skip-downloads", "--skip-verify"])).unwrap();
        let plan = build_section_plan(&options).unwrap();
        let labels = plan
            .iter()
            .flat_map(|section| section.steps.iter().copied())
            .collect::<Vec<_>>();
        for excluded in [
            "Discover Green River public materials",
            "Generate Green River associate tracks",
            "Check official source links",
            "Discover primary official sources",
            "Parse UW major requirement sources",
            "Snapshot Green River annual schedules",
            "Refresh deadline sources",
            "Parse UW Green River equivalency guide",
            "Ingest Green River course catalog",
            "Ingest UW course catalogs",
            "Run closed-loop auto-repair pass",
        ] {
            assert!(!labels.contains(&excluded), "unexpected step: {excluded}");
        }
        assert!(labels.contains(&"Build primary-source automation queue"));
        assert!(labels.contains(&"Build source fingerprints and classify changes"));
        assert!(labels.contains(&"Generate merged course metadata"));
    }

    #[test]
    fn verify_only_plan_runs_only_verification_and_includes_pipeline_validation() {
        let options = build_options(&args(&["--verify-only"])).unwrap();
        let plan = build_section_plan(&options).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].id, "verification");
        assert_eq!(plan[0].steps[0], "Validate source pipeline invariants");
        assert!(!plan[0].steps.contains(&"Run closed-loop auto-repair pass"));
    }

    #[test]
    fn section_plan_option_matrix_matches_cjs_selection_precedence() {
        let verify_options = build_options(&args(&[
            "--verify-only",
            "--only-section=not-a-real-section",
            "--skip-verify",
        ]))
        .unwrap();
        let verify_plan = build_section_plan(&verify_options).unwrap();
        assert_eq!(
            verify_plan
                .iter()
                .map(|section| section.id)
                .collect::<Vec<_>>(),
            ["verification"]
        );
        assert_eq!(
            verify_plan
                .iter()
                .flat_map(|section| &section.steps)
                .count(),
            9
        );
        assert_eq!(
            verify_plan[0].steps[0],
            "Validate source pipeline invariants"
        );

        let isolated_parse_options = build_options(&args(&[
            "--only-section",
            "requirement-parsing",
            "--skip-downloads",
            "--skip-verify",
        ]))
        .unwrap();
        let isolated_parse_plan = build_section_plan(&isolated_parse_options).unwrap();
        assert_eq!(isolated_parse_plan.len(), 1);
        assert_eq!(isolated_parse_plan[0].id, "requirement-parsing");
        assert_eq!(isolated_parse_plan[0].steps.len(), 4);
        assert_eq!(
            isolated_parse_plan[0].steps[0],
            "Prepare source-audit validation prerequisites"
        );
        assert!(
            !isolated_parse_plan[0]
                .steps
                .contains(&"Parse UW major requirement sources")
        );
    }

    #[test]
    fn invalid_or_conflicting_sections_are_rejected() {
        let conflict = build_options(&args(&[
            "--only-section",
            "source-audit",
            "--start-section",
            "verification",
        ]))
        .unwrap_err();
        assert_eq!(
            conflict,
            "Use either --only-section or --start-section, not both."
        );
        let options = build_options(&args(&["--only-section", "unknown"])).unwrap();
        assert!(
            build_section_plan(&options)
                .unwrap_err()
                .contains("Unknown refresh section \"unknown\".")
        );
    }

    #[test]
    fn step_plan_json_retains_javascript_field_order_and_null_options() {
        let options = build_options(&args(&["--only-section=deadline-refresh"])).unwrap();
        let sections = build_section_plan(&options).unwrap();
        let labels = sections
            .iter()
            .flat_map(|section| section.steps.iter().map(|label| (*label).to_string()))
            .collect::<Vec<_>>();
        let json =
            serde_json::to_string(&build_step_plan_json(&options, &sections, &labels)).unwrap();
        assert!(json.starts_with(
            "{\"mode\":\"full-refresh\",\"count\":1,\"labels\":[\"Refresh deadline sources\"]"
        ));
        assert!(json.contains(
            "\"onlySection\":\"deadline-refresh\",\"startSection\":null,\"targetPlanId\":null"
        ));
    }

    #[test]
    fn ts_node_heap_options_match_cjs_filtering_of_empty_environment_values() {
        assert_eq!(
            node_options_with_heap_limit(None),
            "--max-old-space-size=8192"
        );
        assert_eq!(
            node_options_with_heap_limit(Some("")),
            "--max-old-space-size=8192"
        );
        assert_eq!(
            node_options_with_heap_limit(Some("--trace-warnings")),
            "--trace-warnings --max-old-space-size=8192"
        );
    }

    #[test]
    fn source_pipeline_repair_plan_ids_match_javascript_truthiness_and_stringification() {
        let values = serde_json::json!([null, "", [], {}, true, 2]);
        let mut plan_ids = Vec::new();
        append_plan_ids(&mut plan_ids, Some(&values));
        assert_eq!(plan_ids, ["", "[object Object]", "true", "2"]);
        append_plan_ids(&mut plan_ids, Some(&values));
        assert_eq!(plan_ids.len(), 4);
    }

    #[test]
    fn source_pipeline_repair_plan_ids_use_cjs_locale_compare_order() {
        let mut plan_ids = vec![
            "owner-z".to_string(),
            "Owner-a".to_string(),
            "owner-A".to_string(),
            "owner-a".to_string(),
        ];
        plan_ids.sort_by(|left, right| js_locale_compare(left, right));
        assert_eq!(plan_ids, ["owner-a", "owner-A", "Owner-a", "owner-z"]);
    }

    #[test]
    fn schedule_fetch_error_helpers_match_the_cjs_retry_contract() {
        for status in [408, 429, 500, 502, 503, 504] {
            assert!(retryable_http_status(status));
        }
        for status in [200, 400, 404, 501] {
            assert!(!retryable_http_status(status));
        }
        assert_eq!(
            sanitize_fetch_body("  <!DOCTYPE html><html>oops"),
            "[html-error-body]"
        );
        assert_eq!(sanitize_fetch_body("\u{feff} \n body \u{feff}"), "body");
        assert_eq!(
            sanitize_fetch_body("\u{0085}body\u{0085}"),
            "\u{0085}body\u{0085}"
        );
        assert_eq!(
            sanitize_fetch_body("<htmlx <html\u{00a0}error"),
            "[html-error-body]"
        );
        let body = format!("{}Z", "😀".repeat(250));
        assert_eq!(sanitize_fetch_body(&body), "😀".repeat(250));
    }

    #[test]
    fn schedule_fetch_rate_limit_scopes_group_known_institution_domains() {
        assert_eq!(
            rate_limit_scope("https://catalog.greenriver.edu/path"),
            "scope:greenriver.edu"
        );
        assert_eq!(
            rate_limit_scope("https://music.washington.edu/path"),
            "scope:washington.edu"
        );
        assert_eq!(
            rate_limit_scope("https://example.org:8443/path"),
            "host:example.org:8443"
        );
    }
}
