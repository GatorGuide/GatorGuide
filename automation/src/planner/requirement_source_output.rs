use super::requirement_course_codes::CourseCodeParser;
use super::requirement_snapshots::RequirementSnapshotStore;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementSourcePipelineArtifacts {
    pub report_json: String,
    pub parse_markdown: String,
    pub recovery_report_json: String,
    pub recovery_markdown: String,
    pub adapter_typescript: String,
    pub adapter_partition_files: Vec<(String, String)>,
    pub approved_course_filters_typescript: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementSourcePipelineArtifactPaths {
    pub report_json: PathBuf,
    pub parse_markdown: PathBuf,
    pub recovery_report_json: PathBuf,
    pub recovery_markdown: PathBuf,
    pub adapter_typescript: PathBuf,
    pub adapter_partition_directory: PathBuf,
    pub approved_course_filters_typescript: PathBuf,
}

impl RequirementSourcePipelineArtifactPaths {
    pub fn for_source_and_temporary_roots(source_root: &Path, temporary_root: &Path) -> Self {
        let adapter_root = source_root
            .join("constants")
            .join("transfer-planner-source");
        Self {
            report_json: temporary_root
                .join("transfer-planner-requirement-source-parse-report.json"),
            parse_markdown: temporary_root
                .join("transfer-planner-requirement-source-parse-report.md"),
            recovery_report_json: temporary_root
                .join("transfer-planner-parser-recovery-report.json"),
            recovery_markdown: temporary_root.join("transfer-planner-parser-recovery-report.md"),
            adapter_typescript: adapter_root.join("requirement-source-adapters.generated.ts"),
            adapter_partition_directory: adapter_root.join("requirement-source-adapters.generated"),
            approved_course_filters_typescript: adapter_root
                .join("generated-program-approved-course-filters.ts"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RequirementSourcePipelineOutput {
    pub report: Value,
    pub artifacts: RequirementSourcePipelineArtifacts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementSourceAdapterArtifacts {
    pub typescript: String,
    pub partition_files: Vec<(String, String)>,
}

pub fn build_requirement_source_adapter_blocks(
    report: &Value,
    repository_root: &Path,
) -> Vec<Value> {
    report
        .get("owners")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|owner| build_requirement_source_adapter_block(owner, repository_root))
        .collect()
}

pub fn render_requirement_source_adapter_artifacts(
    summary: &Value,
    blocks: &[Value],
) -> Result<RequirementSourceAdapterArtifacts, String> {
    let mut partitions = Vec::with_capacity(blocks.len());
    let mut by_plan = HashMap::<String, Vec<String>>::new();
    for (index, block) in blocks.iter().enumerate() {
        let file_stem = format!("blocks-by-block-id/block-{index:03}.generated");
        let plan_id = block
            .get("planId")
            .filter(|value| !value.is_null())
            .map(typescript_interpolate)
            .unwrap_or_default();
        let plan_id = super::requirement_shape::normalize_js_whitespace(&plan_id);
        let plan_id = if plan_id.is_empty() {
            "unknown".to_string()
        } else {
            plan_id
        };
        let partition_json = serde_json::to_string(&vec![block])
            .map_err(|error| format!("could not serialize source adapter partition: {error}"))?;
        partitions.push((format!("{file_stem}.json"), format!("{partition_json}\n")));
        by_plan.entry(plan_id).or_default().push(file_stem);
    }
    let summary_json = serde_json::to_string_pretty(summary)
        .map_err(|error| format!("could not serialize source adapter summary: {error}"))?;
    let block_partition_key_lines = partitions
        .iter()
        .map(|(file_name, _)| {
            let stem = file_name.strip_suffix(".json").unwrap_or(file_name);
            format!(
                "  {},",
                serde_json::to_string(stem).unwrap_or_else(|_| "\"\"".to_string())
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut plan_ids = by_plan.keys().cloned().collect::<Vec<_>>();
    plan_ids.sort_by(|left, right| locale_compare(left, right));
    let plan_partition_lines = plan_ids
        .iter()
        .filter_map(|plan_id| by_plan.get(plan_id).map(|file_stems| (plan_id, file_stems)))
        .map(|(plan_id, file_stems)| {
            let partition_lines = file_stems
                .iter()
                .map(|stem| {
                    format!(
                        "    {},",
                        serde_json::to_string(stem).unwrap_or_else(|_| "\"\"".to_string())
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "  {}: [\n{}\n  ],",
                serde_json::to_string(plan_id).unwrap_or_else(|_| "\"\"".to_string()),
                partition_lines
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let partition_loader_cases = partitions
        .iter()
        .map(|(file_name, _)| {
            let stem = file_name.strip_suffix(".json").unwrap_or(file_name);
            let quoted = serde_json::to_string(stem).unwrap_or_else(|_| "\"\"".to_string());
            format!(
                "    case {quoted}:\n      return require(\"./requirement-source-adapters.generated/{stem}.json\") as TransferPlannerParsedRequirementSourceBlock[];"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let lines = [
        "/* eslint-disable */",
        "/* auto-generated by scripts/planner/parse-transfer-planner-requirement-sources.cjs */",
        "",
        "import type {",
        "  TransferPlannerParsedRequirementSourceBlock,",
        "  TransferPlannerRequirementSourceAdapterSummary,",
        "} from \"./schema\";",
        "",
        "const { createLazyGeneratedValue } = require(\"./generated-lazy\") as typeof import(\"./generated-lazy\");",
        "",
    ]
    .into_iter()
    .map(str::to_string)
    .chain([
        format!("export const TRANSFER_PLANNER_REQUIREMENT_ADAPTER_SUMMARY = {summary_json} as TransferPlannerRequirementSourceAdapterSummary;"),
        String::new(),
        "const TRANSFER_PLANNER_PARSED_REQUIREMENT_BLOCK_PARTITION_KEYS = [".to_string(),
        block_partition_key_lines,
        "] as const;".to_string(),
        String::new(),
        "const TRANSFER_PLANNER_PARSED_REQUIREMENT_BLOCK_PARTITIONS_BY_PLAN_ID = {".to_string(),
        plan_partition_lines,
        "} as const;".to_string(),
        String::new(),
        "function loadTransferPlannerParsedRequirementBlockPartition(partitionKey: string) {".to_string(),
        "  switch (partitionKey) {".to_string(),
        partition_loader_cases,
        "    default:".to_string(),
        "      return [] as TransferPlannerParsedRequirementSourceBlock[];".to_string(),
        "  }".to_string(),
        "}".to_string(),
        String::new(),
        "function loadTransferPlannerParsedRequirementBlocksForPlanIdPartition(planId: string) {".to_string(),
        "  const partitionKeys =".to_string(),
        "    TRANSFER_PLANNER_PARSED_REQUIREMENT_BLOCK_PARTITIONS_BY_PLAN_ID[".to_string(),
        "      planId as keyof typeof TRANSFER_PLANNER_PARSED_REQUIREMENT_BLOCK_PARTITIONS_BY_PLAN_ID".to_string(),
        "    ] ?? [];".to_string(),
        "  return partitionKeys.flatMap((partitionKey) =>".to_string(),
        "    loadTransferPlannerParsedRequirementBlockPartition(partitionKey)".to_string(),
        "  ) as TransferPlannerParsedRequirementSourceBlock[];".to_string(),
        "}".to_string(),
        String::new(),
        "function loadTransferPlannerParsedRequirementBlocks() {".to_string(),
        "  return TRANSFER_PLANNER_PARSED_REQUIREMENT_BLOCK_PARTITION_KEYS.flatMap((partitionKey) =>".to_string(),
        "    loadTransferPlannerParsedRequirementBlockPartition(partitionKey)".to_string(),
        "  ) as TransferPlannerParsedRequirementSourceBlock[];".to_string(),
        "}".to_string(),
        String::new(),
        "export const TRANSFER_PLANNER_PARSED_REQUIREMENT_BLOCKS =".to_string(),
        "  createLazyGeneratedValue<TransferPlannerParsedRequirementSourceBlock[]>(".to_string(),
        "    loadTransferPlannerParsedRequirementBlocks,".to_string(),
        "    [] as TransferPlannerParsedRequirementSourceBlock[]".to_string(),
        "  );".to_string(),
        String::new(),
        "export function getTransferPlannerParsedRequirementBlocksForPlanId(planId: string) {".to_string(),
        "  return loadTransferPlannerParsedRequirementBlocksForPlanIdPartition(String(planId ?? \"\"));".to_string(),
        "}".to_string(),
        String::new(),
    ])
    .collect::<Vec<_>>()
    .join("\n");
    Ok(RequirementSourceAdapterArtifacts {
        typescript: format!("{lines}\n"),
        partition_files: partitions,
    })
}

fn locale_compare(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| right.cmp(left))
}

fn typescript_interpolate(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => super::requirement_shape::js_number_to_string(value),
        Value::Array(values) => values
            .iter()
            .map(|value| {
                if value.is_null() {
                    String::new()
                } else {
                    typescript_interpolate(value)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

pub fn build_requirement_source_adapter_summary(report: &Value, blocks: &[Value]) -> Value {
    let mut summary = Map::new();
    for key in ["generatedAt", "totalOwners", "okCount", "failedCount"] {
        copy_field(report, &mut summary, key, key);
    }
    summary.insert(
        "parsedRequirementSourceBlockCount".to_string(),
        json!(blocks.len()),
    );
    for key in [
        "parsedRequirementAtomCandidateCount",
        "parsedDegreeMapBlockCandidateCount",
    ] {
        copy_field(report, &mut summary, key, key);
    }
    summary.insert(
        "parsedRequirementGroupCount".to_string(),
        json!(
            blocks
                .iter()
                .map(|block| array_len(block.get("parsedRequirementGroups")))
                .sum::<usize>()
        ),
    );
    copy_field(
        report,
        &mut summary,
        "parsedRequirementCourseCount",
        "parsedRequirementCourseCount",
    );
    for key in [
        "snapshotFallbackCount",
        "countsByAdapterId",
        "countsByAdapterFamily",
        "countsByCampus",
        "countsByResolutionStrategy",
        "countsBySourceRole",
        "countsBySourceRoleStatus",
        "canCreateSchedulableRowCount",
        "canCreateRequiredRowCount",
        "canCreateOptionGroupCount",
        "canCreateApprovedFilterCount",
        "canCreateElectiveListCount",
        "supportOnlySourceCount",
        "nonSchedulableSourceCount",
        "qualityWarningCount",
        "qualityNoteCount",
        "countsByQualitySignalCode",
    ] {
        copy_field(report, &mut summary, key, key);
    }
    Value::Object(summary)
}

pub fn build_requirement_source_adapter_artifacts(
    report: &Value,
    repository_root: &Path,
) -> Result<RequirementSourceAdapterArtifacts, String> {
    let blocks = build_requirement_source_adapter_blocks(report, repository_root);
    let summary = build_requirement_source_adapter_summary(report, &blocks);
    render_requirement_source_adapter_artifacts(&summary, &blocks)
}

pub fn build_requirement_source_pipeline_output(
    owners: &[Value],
    parse_options: &Value,
    markdown_options: &Value,
    repository_root: &Path,
    snapshot_store: &RequirementSnapshotStore,
    course_parser: &CourseCodeParser,
) -> Result<RequirementSourcePipelineOutput, String> {
    let report = super::requirement_parse_reports::build_parse_report(owners, parse_options);
    let artifacts = build_requirement_source_pipeline_artifacts(
        &report,
        markdown_options,
        repository_root,
        snapshot_store,
        course_parser,
    )?;
    Ok(RequirementSourcePipelineOutput { report, artifacts })
}

pub fn build_requirement_source_pipeline_artifacts(
    report: &Value,
    markdown_options: &Value,
    repository_root: &Path,
    snapshot_store: &RequirementSnapshotStore,
    course_parser: &CourseCodeParser,
) -> Result<RequirementSourcePipelineArtifacts, String> {
    let adapter_artifacts = build_requirement_source_adapter_artifacts(report, repository_root)?;
    let approved_course_filters_typescript = super::requirement_approved_filter_output::build_generated_program_approved_course_filters_typescript(
        report,
        snapshot_store,
        course_parser,
    )?;
    let recovery_report = report
        .get("parserRecoveryReport")
        .cloned()
        .unwrap_or(Value::Null);
    Ok(RequirementSourcePipelineArtifacts {
        report_json: super::planner_reporting::serialize_json_report(report)?,
        parse_markdown:
            super::requirement_parse_markdown::build_requirement_source_parse_markdown_report(
                report,
                markdown_options,
            ),
        recovery_report_json: super::planner_reporting::serialize_json_report(&recovery_report)?,
        recovery_markdown: super::requirement_reports::build_parser_recovery_markdown_report(
            &recovery_report,
        ),
        adapter_typescript: adapter_artifacts.typescript,
        adapter_partition_files: adapter_artifacts.partition_files,
        approved_course_filters_typescript,
    })
}

pub fn write_requirement_source_pipeline_artifacts(
    artifacts: &RequirementSourcePipelineArtifacts,
    paths: &RequirementSourcePipelineArtifactPaths,
) -> Result<(), String> {
    let mut partition_paths = Vec::with_capacity(artifacts.adapter_partition_files.len());
    let mut expected_partition_files =
        HashSet::with_capacity(artifacts.adapter_partition_files.len());
    for (relative_path, contents) in &artifacts.adapter_partition_files {
        let relative_path = Path::new(relative_path);
        if relative_path.is_absolute()
            || relative_path.to_string_lossy().contains(':')
            || relative_path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::CurDir
                )
            })
        {
            return Err(format!(
                "adapter partition path must be relative to its output directory: {}",
                relative_path.display()
            ));
        }
        let output_path = paths.adapter_partition_directory.join(relative_path);
        if !expected_partition_files.insert(output_path.clone()) {
            return Err(format!(
                "duplicate adapter partition output path: {}",
                output_path.display()
            ));
        }
        partition_paths.push((output_path, contents));
    }

    write_requirement_source_output_file(&paths.report_json, &artifacts.report_json)?;
    write_requirement_source_output_file(&paths.parse_markdown, &artifacts.parse_markdown)?;
    write_requirement_source_output_file(
        &paths.recovery_report_json,
        &artifacts.recovery_report_json,
    )?;
    write_requirement_source_output_file(&paths.recovery_markdown, &artifacts.recovery_markdown)?;
    write_requirement_source_output_file(&paths.adapter_typescript, &artifacts.adapter_typescript)?;
    write_requirement_source_output_file(
        &paths.approved_course_filters_typescript,
        &artifacts.approved_course_filters_typescript,
    )?;
    for (output_path, contents) in partition_paths {
        write_requirement_source_output_file(&output_path, contents)?;
    }

    remove_stale_requirement_source_adapter_partitions(
        &paths.adapter_partition_directory.join("blocks-by-block-id"),
        &expected_partition_files,
    )
}

fn write_requirement_source_output_file(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "could not create output directory {}: {error}",
                parent.display()
            )
        })?;
    }
    fs::write(path, contents)
        .map_err(|error| format!("could not write output file {}: {error}", path.display()))
}

fn remove_stale_requirement_source_adapter_partitions(
    directory: &Path,
    expected_files: &HashSet<PathBuf>,
) -> Result<(), String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "could not read adapter partition directory {}: {error}",
                directory.display()
            ));
        }
    };
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "could not read adapter partition directory {}: {error}",
                directory.display()
            )
        })?;
        let path = entry.path();
        if !path.is_file()
            || !is_managed_requirement_source_adapter_partition(&path)
            || expected_files.contains(&path)
        {
            continue;
        }
        fs::remove_file(&path).map_err(|error| {
            format!(
                "could not remove stale adapter partition {}: {error}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn is_managed_requirement_source_adapter_partition(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(index) = file_name
        .strip_prefix("block-")
        .and_then(|name| name.strip_suffix(".generated.json"))
    else {
        return false;
    };
    !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
}

fn build_requirement_source_adapter_block(owner: &Value, repository_root: &Path) -> Value {
    let mut block = Map::new();
    block.insert(
        "id".to_string(),
        json!(super::requirement_ids::build_requirement_source_block_id(
            owner
        )),
    );
    for key in ["ownerId", "ownerTitle", "planId"] {
        copy_field(owner, &mut block, key, key);
    }
    block.insert(
        "pathwayId".to_string(),
        owner
            .get("pathwayId")
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or(Value::Null),
    );
    for key in [
        "campusId",
        "primaryParserType",
        "primarySourceUrl",
        "primarySourceLabel",
        "parserType",
        "adapterId",
        "adapterFamily",
        "sourceUrl",
    ] {
        copy_field(owner, &mut block, key, key);
    }
    if let Some(value) = owner
        .get("coveredSourceUrls")
        .filter(|value| value_length(value) > 0)
    {
        block.insert("coveredSourceUrls".to_string(), value.clone());
    }
    for key in ["sourceLabel", "sourceRole"] {
        copy_field(owner, &mut block, key, key);
    }
    block.insert(
        "sourceRoleStatus".to_string(),
        owner
            .get("sourceRoleStatus")
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or_else(|| {
                let role = owner
                    .get("sourceRole")
                    .filter(|value| !value.is_null())
                    .map(typescript_interpolate)
                    .unwrap_or_else(|| "ignored".to_string());
                json!(
                    super::requirement_scope::get_requirement_source_role_status(Some(&role))
                        .as_str()
                )
            }),
    );
    block.insert(
        "canCreateSchedulableRows".to_string(),
        json!(
            owner
                .get("canCreateSchedulableRows")
                .and_then(Value::as_bool)
                != Some(false)
        ),
    );
    copy_field(owner, &mut block, "sourceScope", "sourceScope");
    for key in [
        "canCreateRequiredRows",
        "canCreateOptionGroups",
        "canCreateCreditBuckets",
        "canCreateCategoryOptions",
        "canCreateApprovedFilters",
        "canCreateElectiveLists",
        "canCreateSequencingHints",
        "canCreateAdmissionPrepRows",
        "canCreateScheduleRows",
        "supportOnly",
        "nonSchedulable",
    ] {
        block.insert(
            key.to_string(),
            json!(owner.get(key).and_then(Value::as_bool) == Some(true)),
        );
    }
    copy_field(
        owner,
        &mut block,
        "sourceSectionAudit",
        "sourceSectionAudit",
    );
    for key in [
        "sourceSectionFilterAuditRows",
        "sourceSectionFilterAuditLines",
        "parserSequenceChoiceAuditRows",
        "parserSequenceChoiceAuditLines",
        "sourceScopeAuditLines",
    ] {
        copy_nullish_array(owner, &mut block, key);
    }
    for key in [
        "resolutionStrategy",
        "ok",
        "parseConfidence",
        "parsedUwCourseCodes",
    ] {
        copy_field(owner, &mut block, key, key);
    }
    for key in [
        "approvedFilterUwCourseCodes",
        "electiveListUwCourseCodes",
        "supportOnlyUwCourseCodes",
        "supportLists",
    ] {
        copy_nullish_array(owner, &mut block, key);
    }
    for key in [
        "sourceOnlyUwCourseCodes",
        "structuredOnlyUwCourseCodes",
        "requirementCueLines",
        "chooseStatements",
        "pathwayLabels",
        "qualitySignals",
    ] {
        copy_field(owner, &mut block, key, key);
    }
    if let Some(value) = owner.get("parserRecovery").filter(|value| !value.is_null()) {
        block.insert("parserRecovery".to_string(), value.clone());
    }
    for key in [
        "parsedRequirementAtomCandidates",
        "parsedDegreeMapBlockCandidates",
    ] {
        copy_field(owner, &mut block, key, key);
    }
    for key in [
        "parsedRequirementGroups",
        "parsedRequirementCourses",
        "parsedRequirementReplacements",
    ] {
        if let Some(value) = owner.get(key).filter(|value| value_length(value) > 0) {
            block.insert(key.to_string(), value.clone());
        }
    }
    block.insert(
        "snapshotPath".to_string(),
        owner
            .get("snapshotPath")
            .filter(|value| truthy(value))
            .and_then(Value::as_str)
            .map(|path| relative_display_path(repository_root, Path::new(path)))
            .map(Value::String)
            .unwrap_or(Value::Null),
    );
    for key in ["usedSnapshotFallback", "snapshotFallbackReason", "error"] {
        copy_field(owner, &mut block, key, key);
    }
    Value::Object(block)
}

fn copy_nullish_array(source: &Value, destination: &mut Map<String, Value>, key: &str) {
    destination.insert(
        key.to_string(),
        source
            .get(key)
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or_else(|| json!([])),
    );
}

fn copy_field(
    source: &Value,
    destination: &mut Map<String, Value>,
    source_key: &str,
    output_key: &str,
) {
    if let Some(value) = source.get(source_key) {
        destination.insert(output_key.to_string(), value.clone());
    }
}

fn value_length(value: &Value) -> usize {
    match value {
        Value::Array(values) => values.len(),
        Value::String(value) => value.encode_utf16().count(),
        _ => 0,
    }
}

fn array_len(value: Option<&Value>) -> usize {
    value.map(value_length).unwrap_or(0)
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn relative_display_path(root: &Path, path: &Path) -> String {
    let resolved_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let relative = resolved_path.strip_prefix(root).unwrap_or(&resolved_path);
    relative.to_string_lossy().replace('\\', "/")
}
