use chrono::{SecondsFormat, Utc};
use regex::Regex;
use reqwest::blocking::Client;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};
use url::Url;

const CATALOG_YEAR_LABEL: &str = "current-uw-course-catalog";
const USER_AGENT: &str = "GatorGuideTransferPlannerCatalogIngest/1.0";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const HOST_MIN_DELAY: Duration = Duration::from_millis(1_500);
const MAX_RETRIES: usize = 1;
const RETRY_DELAY: Duration = Duration::from_millis(500);

const CAMPUSES: [CampusConfig; 3] = [
    CampusConfig {
        school_id: "uw-seattle",
        label: "UW Seattle",
        index_url: "https://www.washington.edu/students/crscat/",
    },
    CampusConfig {
        school_id: "uw-bothell",
        label: "UW Bothell",
        index_url: "https://www.washington.edu/students/crscatb/",
    },
    CampusConfig {
        school_id: "uw-tacoma",
        label: "UW Tacoma",
        index_url: "https://www.washington.edu/students/crscatt/",
    },
];

const INVALID_PLANNER_SUBJECT_TOKENS: &[&str] = &[
    "ADD", "ALSO", "APPLY", "ARE", "AUTUMN", "BOTH", "BUT", "BY", "CALL", "COMPLETE", "CONSIDER",
    "CORE", "COURSE", "COURSES", "CREDIT", "DOES", "EITHER", "FOUR", "EXCLUDES", "FROM", "GRADED",
    "HAVE", "HAS", "IF", "INCLUDE", "INCLUDES", "IS", "JUST", "LANGUAGE", "LIKE", "MAX", "MORE",
    "MUST", "NOT", "OFFICE", "ONLY", "REQUIRE", "REQUIRED", "ROOM", "SECTION", "SEPARATE",
    "TAKING", "THAN", "THROUGH", "TOOK", "USE", "WANT", "WHILE", "WILL", "WINTER", "SPR",
];

#[derive(Clone, Copy)]
struct CampusConfig {
    school_id: &'static str,
    label: &'static str,
    index_url: &'static str,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UwSubjectInfo {
    pub subject_code: String,
    pub label: String,
    pub url: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UwCatalogEntry {
    pub school_id: String,
    pub code: String,
    pub title: String,
    pub credit_value: Option<serde_json::Number>,
    pub credit_label: Option<String>,
    pub catalog_description: Option<String>,
    pub prerequisite_notes: Vec<String>,
    pub corequisite_notes: Vec<String>,
    pub effective_year_ranges: Vec<UwEffectiveYearRange>,
    pub source_links: Vec<UwSourceLink>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UwEffectiveYearRange {
    pub start_label: String,
    pub end_label: Option<String>,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UwSourceLink {
    pub label: String,
    pub url: String,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UwParsedSubjectPage {
    pub subject_code: String,
    pub url: String,
    pub wanted_course_count: usize,
    pub parsed_course_count: usize,
    pub source_fingerprint: String,
    pub used_snapshot_fallback: bool,
    pub snapshot_fallback_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InvalidPlannerSubjectCode {
    pub subject_code: String,
    pub raw_subject_labels: Vec<String>,
    pub wanted_course_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UwCampusReport {
    pub school_id: String,
    pub label: String,
    pub index_url: String,
    pub index_snapshot_path: String,
    pub source_fingerprint: String,
    pub used_index_snapshot_fallback: bool,
    pub index_snapshot_fallback_reason: Option<String>,
    pub subject_snapshot_fallback_count: usize,
    pub relevant_course_count: usize,
    pub subject_count: usize,
    pub parsed_subject_pages: Vec<UwParsedSubjectPage>,
    pub missing_subject_codes: Vec<String>,
    pub invalid_planner_subject_codes: Vec<InvalidPlannerSubjectCode>,
    pub entries: Vec<UwCatalogEntry>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UwCatalogReport {
    pub generated_at: String,
    pub catalog_year_label: String,
    pub relevant_course_count: usize,
    pub course_count: usize,
    pub courses_with_credit_labels: usize,
    pub courses_with_prerequisite_notes: usize,
    pub courses_with_corequisite_notes: usize,
    pub invalid_planner_subject_code_count: usize,
    pub campuses: Vec<UwCampusReport>,
    pub entries: Vec<UwCatalogEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedCourseHeading {
    pub code: String,
    pub title: String,
    pub credit_label: Option<String>,
    pub credit_value: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequirementNotes {
    pub prerequisite_notes: Vec<String>,
    pub corequisite_notes: Vec<String>,
}

#[derive(Clone, Debug)]
struct SnapshotText {
    html: String,
    used_snapshot_fallback: bool,
    snapshot_fallback_reason: Option<String>,
}

#[derive(Default)]
struct FetchState {
    last_request: Option<Instant>,
}

#[derive(Default)]
struct SubjectRequest {
    wanted_course_codes: BTreeSet<String>,
    raw_subject_labels: BTreeSet<String>,
}

pub fn refresh(repository_root: &Path, cache_only: bool) -> Result<UwCatalogReport, String> {
    let source_root = repository_root.join("source");
    let snapshot_dir = source_root.join(".tmp/snapshots/transfer-planner-catalog-snapshots");
    let report_dir = source_root.join(".tmp/reports");
    let report_json_path = report_dir.join("transfer-planner-uw-catalog-ingest.json");
    let report_md_path = report_dir.join("transfer-planner-uw-catalog-ingest.md");
    fs::create_dir_all(&snapshot_dir)
        .map_err(|error| format!("could not create UW catalog snapshot directory: {error}"))?;
    fs::create_dir_all(&report_dir)
        .map_err(|error| format!("could not create UW catalog report directory: {error}"))?;

    let relevant_codes = export_relevant_course_codes(repository_root)?;
    let client = build_client()?;
    let mut fetch_state = FetchState::default();
    let mut campuses = Vec::with_capacity(CAMPUSES.len());
    for config in CAMPUSES {
        campuses.push(ingest_campus(
            config,
            relevant_codes
                .get(config.school_id)
                .cloned()
                .unwrap_or_default(),
            &snapshot_dir,
            &client,
            &mut fetch_state,
            cache_only,
        )?);
    }
    let mut entries: Vec<UwCatalogEntry> = campuses
        .iter()
        .flat_map(|campus| campus.entries.clone())
        .collect();
    entries.sort_by(|left, right| {
        left.school_id
            .cmp(&right.school_id)
            .then_with(|| left.code.cmp(&right.code))
    });
    let relevant_course_count = relevant_codes.values().map(BTreeSet::len).sum();
    let report = UwCatalogReport {
        generated_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        catalog_year_label: CATALOG_YEAR_LABEL.to_string(),
        relevant_course_count,
        course_count: entries.len(),
        courses_with_credit_labels: entries
            .iter()
            .filter(|entry| entry.credit_label.is_some())
            .count(),
        courses_with_prerequisite_notes: entries
            .iter()
            .filter(|entry| !entry.prerequisite_notes.is_empty())
            .count(),
        courses_with_corequisite_notes: entries
            .iter()
            .filter(|entry| !entry.corequisite_notes.is_empty())
            .count(),
        invalid_planner_subject_code_count: campuses
            .iter()
            .map(|campus| campus.invalid_planner_subject_codes.len())
            .sum(),
        campuses,
        entries,
    };
    write_report(&report, &report_json_path, &report_md_path)?;
    Ok(report)
}

fn export_relevant_course_codes(
    repository_root: &Path,
) -> Result<HashMap<String, BTreeSet<String>>, String> {
    let script =
        repository_root.join("source/scripts/planner/export-transfer-planner-uw-course-codes.cjs");
    if !script.is_file() {
        return Err(format!(
            "UW course-code bridge not found: {}",
            script.display()
        ));
    }
    let output = Command::new("node")
        .arg(script)
        .current_dir(repository_root.join("source"))
        .output()
        .map_err(|error| format!("could not start UW course-code bridge: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("UW course-code bridge failed: {}", stderr.trim()));
    }
    let parsed: BTreeMap<String, Vec<String>> = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("UW course-code bridge returned invalid JSON: {error}"))?;
    Ok(parsed
        .into_iter()
        .map(|(school, codes)| (school, codes.into_iter().collect()))
        .collect())
}

fn build_client() -> Result<Client, String> {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("redirect limit exceeded");
        }
        let host = attempt
            .url()
            .host_str()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if attempt.url().scheme() == "https"
            && (host == "washington.edu" || host.ends_with(".washington.edu"))
        {
            attempt.follow()
        } else {
            attempt.stop()
        }
    });
    Client::builder()
        .user_agent(USER_AGENT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(policy)
        .build()
        .map_err(|error| format!("could not configure UW catalog client: {error}"))
}

fn fetch_text_with_snapshot(
    client: &Client,
    state: &mut FetchState,
    url: &str,
    snapshot_path: &Path,
    label: &str,
    cache_only: bool,
) -> Result<SnapshotText, String> {
    if cache_only {
        return read_snapshot(
            snapshot_path,
            format!("cache-only mode requires the cached {label} snapshot"),
        );
    }
    match fetch_text(client, state, url) {
        Ok(html) => {
            fs::write(snapshot_path, html.as_bytes()).map_err(|error| {
                format!(
                    "could not write UW catalog snapshot {}: {error}",
                    snapshot_path.display()
                )
            })?;
            Ok(SnapshotText {
                html,
                used_snapshot_fallback: false,
                snapshot_fallback_reason: None,
            })
        }
        Err(error) => {
            let mut snapshot = read_snapshot(snapshot_path, error.clone())?;
            snapshot.used_snapshot_fallback = true;
            snapshot.snapshot_fallback_reason = Some(error);
            eprintln!(
                "Live {label} fetch failed; using cached snapshot at {}.",
                snapshot_path.display()
            );
            Ok(snapshot)
        }
    }
}

fn read_snapshot(path: &Path, reason: String) -> Result<SnapshotText, String> {
    let html = fs::read_to_string(path).map_err(|error| {
        format!(
            "{reason}; no readable snapshot at {}: {error}",
            path.display()
        )
    })?;
    Ok(SnapshotText {
        html,
        used_snapshot_fallback: false,
        snapshot_fallback_reason: None,
    })
}

fn fetch_text(client: &Client, state: &mut FetchState, url: &str) -> Result<String, String> {
    if let Some(last) = state.last_request {
        let elapsed = last.elapsed();
        if elapsed < HOST_MIN_DELAY {
            thread::sleep(HOST_MIN_DELAY - elapsed);
        }
    }
    state.last_request = Some(Instant::now());
    let mut last_error = None;
    for attempt in 0..=MAX_RETRIES {
        match fetch_text_once(client, url) {
            Ok(html) => return Ok(html),
            Err(error) => {
                last_error = Some(error);
                if attempt < MAX_RETRIES {
                    thread::sleep(RETRY_DELAY);
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| format!("UW catalog fetch failed for {url}")))
}

fn fetch_text_once(client: &Client, url: &str) -> Result<String, String> {
    let response = client
        .get(url)
        .send()
        .map_err(|error| format!("UW catalog fetch failed for {url}: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "UW catalog fetch failed for {url}: HTTP {}",
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES)
    {
        return Err(format!(
            "UW catalog response exceeded {MAX_RESPONSE_BYTES} bytes: {url}"
        ));
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read UW catalog response from {url}: {error}"))?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(format!(
            "UW catalog response exceeded {MAX_RESPONSE_BYTES} bytes: {url}"
        ));
    }
    String::from_utf8(bytes)
        .map_err(|error| format!("UW catalog response was not UTF-8 ({url}): {error}"))
}

fn ingest_campus(
    config: CampusConfig,
    relevant_codes: BTreeSet<String>,
    snapshot_dir: &Path,
    client: &Client,
    fetch_state: &mut FetchState,
    cache_only: bool,
) -> Result<UwCampusReport, String> {
    let index_snapshot_path = snapshot_dir.join(format!("{}-course-index.html", config.school_id));
    let index_snapshot = fetch_text_with_snapshot(
        client,
        fetch_state,
        config.index_url,
        &index_snapshot_path,
        &format!("{} course index", config.label),
        cache_only,
    )?;
    let subject_map = build_subject_url_map(&index_snapshot.html, config.index_url)?;
    let known_subject_codes: HashSet<String> = subject_map.keys().cloned().collect();
    let mut wanted_by_subject = BTreeMap::<String, SubjectRequest>::new();
    for code in &relevant_codes {
        let raw_subject = get_raw_subject_label(code);
        let subject = normalize_subject_code(&raw_subject);
        let request = wanted_by_subject.entry(subject).or_default();
        request.wanted_course_codes.insert(code.clone());
        request.raw_subject_labels.insert(raw_subject);
    }

    let mut entries = Vec::new();
    let mut missing_subject_codes = Vec::new();
    let mut invalid_planner_subject_codes = Vec::new();
    let mut parsed_subject_pages = Vec::new();
    for (subject_code, request) in &wanted_by_subject {
        let Some(subject_info) = subject_map.get(subject_code) else {
            let raw_subject_labels: Vec<String> =
                request.raw_subject_labels.iter().cloned().collect();
            if raw_subject_labels
                .iter()
                .any(|label| is_likely_invalid_planner_subject(label, &known_subject_codes))
            {
                invalid_planner_subject_codes.push(InvalidPlannerSubjectCode {
                    subject_code: subject_code.clone(),
                    raw_subject_labels,
                    wanted_course_count: request.wanted_course_codes.len(),
                });
            } else {
                missing_subject_codes.push(subject_code.clone());
            }
            continue;
        };
        let snapshot_path = snapshot_dir.join(format!(
            "{}-{}.html",
            config.school_id,
            subject_code.to_ascii_lowercase()
        ));
        let snapshot = fetch_text_with_snapshot(
            client,
            fetch_state,
            &subject_info.url,
            &snapshot_path,
            &format!("{} {subject_code} catalog page", config.label),
            cache_only,
        )?;
        let subject_entries = parse_subject_page(
            &snapshot.html,
            config,
            subject_info,
            &request.wanted_course_codes,
        )?;
        entries.extend(subject_entries.iter().cloned());
        parsed_subject_pages.push(UwParsedSubjectPage {
            subject_code: subject_code.clone(),
            url: subject_info.url.clone(),
            wanted_course_count: request.wanted_course_codes.len(),
            parsed_course_count: subject_entries.len(),
            source_fingerprint: sha256_text(&snapshot.html),
            used_snapshot_fallback: snapshot.used_snapshot_fallback,
            snapshot_fallback_reason: snapshot.snapshot_fallback_reason,
        });
    }
    Ok(UwCampusReport {
        school_id: config.school_id.to_string(),
        label: config.label.to_string(),
        index_url: config.index_url.to_string(),
        index_snapshot_path: index_snapshot_path.display().to_string(),
        source_fingerprint: sha256_text(&index_snapshot.html),
        used_index_snapshot_fallback: index_snapshot.used_snapshot_fallback,
        index_snapshot_fallback_reason: index_snapshot.snapshot_fallback_reason,
        subject_snapshot_fallback_count: parsed_subject_pages
            .iter()
            .filter(|page| page.used_snapshot_fallback)
            .count(),
        relevant_course_count: relevant_codes.len(),
        subject_count: wanted_by_subject.len(),
        parsed_subject_pages,
        missing_subject_codes,
        invalid_planner_subject_codes,
        entries,
    })
}

pub fn build_subject_url_map(
    html: &str,
    index_url: &str,
) -> Result<BTreeMap<String, UwSubjectInfo>, String> {
    let base =
        Url::parse(index_url).map_err(|error| format!("invalid UW catalog index URL: {error}"))?;
    let anchor_pattern = Regex::new(r#"(?is)<a\s+href="([^"]+)"[^>]*>(.*?)</a>"#)
        .map_err(|error| error.to_string())?;
    let subject_pattern = Regex::new(r"\(([A-Z][A-Z &]{0,14})\)\s*$").unwrap();
    let mut subjects = BTreeMap::new();
    for anchor in anchor_pattern.captures_iter(html) {
        let href = decode_html_entities(anchor.get(1).unwrap().as_str());
        let label = strip_html(anchor.get(2).unwrap().as_str());
        let Some(captures) = subject_pattern.captures(&label) else {
            continue;
        };
        let subject_code = normalize_subject_code(captures.get(1).unwrap().as_str());
        let url = base
            .join(&href)
            .map_err(|error| format!("invalid UW subject link {href:?}: {error}"))?
            .to_string();
        subjects.insert(
            subject_code.clone(),
            UwSubjectInfo {
                subject_code,
                label,
                url,
            },
        );
    }
    Ok(subjects)
}

pub fn normalize_course_code(value: &str) -> String {
    let normalized = normalize_whitespace(value).to_ascii_uppercase();
    let pattern = Regex::new(r"^([A-Z][A-Z &]{0,14})\s+(\d{3}(?:\.\d+)?[A-Z]?)$").unwrap();
    if let Some(captures) = pattern.captures(&normalized) {
        format!(
            "{} {}",
            captures
                .get(1)
                .unwrap()
                .as_str()
                .split_whitespace()
                .collect::<String>(),
            captures.get(2).unwrap().as_str()
        )
    } else {
        normalized.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}

pub fn normalize_subject_code(value: &str) -> String {
    normalize_whitespace(value)
        .to_ascii_uppercase()
        .split_whitespace()
        .collect()
}

fn get_raw_subject_label(course_code: &str) -> String {
    let pattern = Regex::new(r"\s+\d.*$").unwrap();
    normalize_whitespace(&pattern.replace(course_code, ""))
}

pub fn is_likely_invalid_planner_subject(
    raw_subject_label: &str,
    known_subject_codes: &HashSet<String>,
) -> bool {
    let tokens: Vec<String> = normalize_whitespace(raw_subject_label)
        .to_ascii_uppercase()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let collapsed = tokens.join("");
    if tokens.is_empty() {
        return true;
    }
    if known_subject_codes.contains(&collapsed) {
        return false;
    }
    if tokens.len() > 2
        || tokens
            .iter()
            .any(|token| INVALID_PLANNER_SUBJECT_TOKENS.contains(&token.as_str()))
    {
        return true;
    }
    tokens.len() == 1
        && INVALID_PLANNER_SUBJECT_TOKENS
            .iter()
            .any(|token| collapsed.starts_with(token) && collapsed.len() > token.len())
}

pub fn extract_requirement_notes(body_text: &str) -> RequirementNotes {
    let mut notes = RequirementNotes::default();
    if let Some(prerequisite) = capture_requirement(
        body_text,
        r"(?is)Prerequisite:\s*(.*?)\s+(?:Offered:|Recommended:|Course|View course details|$)",
    ) {
        notes
            .prerequisite_notes
            .push(format!("Official UW prerequisite text: {prerequisite}"));
        notes.prerequisite_notes.push("prerequisite text is preserved as a note until a parser can safely normalize AND/OR/minimum-grade semantics into graph prerequisites.".to_string());
    }
    if let Some(recommended) = capture_requirement(
        body_text,
        r"(?is)Recommended:\s*(.*?)\s+(?:Offered:|Prerequisite:|Course|View course details|$)",
    ) {
        notes.prerequisite_notes.push(format!(
            "Official UW recommended-preparation text: {recommended}"
        ));
    }
    if let Some(corequisite) = capture_requirement(
        body_text,
        r"(?is)(?:Corequisite:|Must be taken with)\s*(.*?)\s+(?:Offered:|Prerequisite:|Recommended:|Course|View course details|$)",
    ) {
        notes
            .corequisite_notes
            .push(format!("Official UW corequisite text: {corequisite}"));
    }
    notes
}

fn capture_requirement(text: &str, pattern: &str) -> Option<String> {
    Regex::new(pattern)
        .ok()?
        .captures(text)
        .and_then(|captures| captures.get(1))
        .map(|value| normalize_whitespace(value.as_str()))
        .filter(|value| !value.is_empty())
}

pub fn parse_credit_label(value: &str) -> Option<String> {
    let pattern =
        Regex::new(r"(?i)(\*|\d+(?:\.\d+)?(?:-\d+(?:\.\d+)?)?(?:,\s*max\.\s*\d+)?)$").unwrap();
    pattern
        .captures(value)
        .and_then(|captures| captures.get(1))
        .map(|value| normalize_whitespace(value.as_str()))
}

pub fn parse_credit_value(credit_label: Option<&str>) -> Option<f64> {
    let pattern = Regex::new(r"^(\d+(?:\.\d+)?)$").unwrap();
    let captures = pattern.captures(credit_label?)?;
    captures.get(1)?.as_str().parse().ok()
}

fn parse_credit_json_number(credit_value: &str) -> Option<serde_json::Number> {
    let value: f64 = credit_value.parse().ok()?;
    if value.fract() == 0.0 && value >= i64::MIN as f64 && value < i64::MAX as f64 {
        Some(serde_json::Number::from(value as i64))
    } else {
        serde_json::Number::from_f64(value)
    }
}

pub fn parse_course_heading(heading_text: &str) -> Option<ParsedCourseHeading> {
    let normalized = normalize_whitespace(heading_text);
    let course_pattern =
        Regex::new(r"^([A-Z][A-Z &]{0,14}\s+\d{3}(?:\.\d+)?[A-Z]?)\s+(.+)$").unwrap();
    let course = course_pattern.captures(&normalized)?;
    let code = normalize_course_code(course.get(1)?.as_str());
    let tail = course.get(2)?.as_str();
    let credit_pattern = Regex::new(r"\(([^)]*?)\)(?:\s+[A-Z][A-Za-z&, ]*)?$").unwrap();
    let credit = credit_pattern.captures(tail);
    let credit_label = credit
        .as_ref()
        .and_then(|captures| captures.get(1))
        .and_then(|value| parse_credit_label(value.as_str()));
    let title = if let Some(credit) = credit.as_ref().and_then(|captures| captures.get(0)) {
        normalize_whitespace(&tail[..credit.start()])
    } else {
        normalize_whitespace(tail)
    };
    let credit_value = parse_credit_value(credit_label.as_deref()).map(|value| value.to_string());
    Some(ParsedCourseHeading {
        code,
        title,
        credit_label,
        credit_value,
    })
}

fn parse_subject_page(
    html: &str,
    config: CampusConfig,
    subject: &UwSubjectInfo,
    wanted_course_codes: &BTreeSet<String>,
) -> Result<Vec<UwCatalogEntry>, String> {
    let course_pattern = Regex::new(r#"(?is)<a\s+name="[^"]*">\s*<p>\s*<b>(.*?)</b><br\s*/?>(.*?)(?:<br\s*/?>\s*<a\s+href=|</p>)"#)
        .map_err(|error| error.to_string())?;
    let mut entries = Vec::new();
    for captures in course_pattern.captures_iter(html) {
        let heading = strip_html(
            captures
                .get(1)
                .map(|value| value.as_str())
                .unwrap_or_default(),
        );
        let Some(heading) = parse_course_heading(&heading) else {
            continue;
        };
        if !wanted_course_codes.contains(&heading.code) {
            continue;
        }
        let body_text = strip_html(
            captures
                .get(2)
                .map(|value| value.as_str())
                .unwrap_or_default(),
        );
        let requirements = extract_requirement_notes(&body_text);
        let anchor = heading
            .code
            .to_ascii_lowercase()
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .collect::<String>();
        entries.push(UwCatalogEntry {
            school_id: config.school_id.to_string(),
            code: heading.code.clone(),
            title: heading.title,
            credit_value: heading
                .credit_value
                .as_deref()
                .and_then(parse_credit_json_number),
            credit_label: heading.credit_label,
            catalog_description: if body_text.is_empty() {
                None
            } else {
                Some(body_text)
            },
            prerequisite_notes: requirements.prerequisite_notes,
            corequisite_notes: requirements.corequisite_notes,
            effective_year_ranges: vec![UwEffectiveYearRange {
                start_label: CATALOG_YEAR_LABEL.to_string(),
                end_label: None,
                note: format!(
                    "Parsed from the official {} course descriptions page.",
                    config.label
                ),
            }],
            source_links: vec![UwSourceLink {
                label: format!(
                    "{} course descriptions ({})",
                    config.label, subject.subject_code
                ),
                url: format!("{}#{anchor}", subject.url),
                note: "Course detail parsed from the official UW course descriptions page."
                    .to_string(),
            }],
            notes: vec![format!(
                "{} catalog metadata parsed from the official UW course descriptions.",
                config.label
            )],
        });
    }
    Ok(entries)
}

fn strip_html(value: &str) -> String {
    let tags = Regex::new(r"(?s)<[^>]+>").unwrap();
    normalize_whitespace(&tags.replace_all(value, " "))
}

fn decode_html_entities(value: &str) -> String {
    let entities = Regex::new(r"(?i)&(amp|apos|nbsp|quot|#39|#160|#8211|#8212|#8217);").unwrap();
    entities
        .replace_all(value, |captures: &regex::Captures<'_>| {
            match captures
                .get(1)
                .unwrap()
                .as_str()
                .to_ascii_lowercase()
                .as_str()
            {
                "amp" => "&",
                "apos" | "#39" | "#8217" => "'",
                "nbsp" | "#160" => " ",
                "quot" => "\"",
                "#8211" | "#8212" => "-",
                _ => captures.get(0).unwrap().as_str(),
            }
            .to_string()
        })
        .into_owned()
}

fn normalize_whitespace(value: &str) -> String {
    decode_html_entities(value)
        .replace('\u{00a0}', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn sha256_text(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn write_report(
    report: &UwCatalogReport,
    json_path: &Path,
    markdown_path: &Path,
) -> Result<(), String> {
    let mut encoded = serde_json::to_string_pretty(report)
        .map_err(|error| format!("could not encode UW catalog report: {error}"))?;
    encoded.push('\n');
    fs::write(json_path, encoded).map_err(|error| {
        format!(
            "could not write UW catalog report {}: {error}",
            json_path.display()
        )
    })?;
    let mut lines = vec![
        "# UW Catalog Ingest".to_string(),
        "".to_string(),
        format!("Generated: {}", report.generated_at),
        "".to_string(),
        format!(
            "- Planner-relevant UW courses: {}",
            report.relevant_course_count
        ),
        format!("- Parsed UW catalog courses: {}", report.course_count),
        format!(
            "- Courses with credit labels: {}",
            report.courses_with_credit_labels
        ),
        format!(
            "- Courses with prerequisite notes: {}",
            report.courses_with_prerequisite_notes
        ),
        format!(
            "- Courses with corequisite notes: {}",
            report.courses_with_corequisite_notes
        ),
        format!(
            "- Ignored invalid planner subject tokens: {}",
            report.invalid_planner_subject_code_count
        ),
        "".to_string(),
        "## Campuses".to_string(),
        "".to_string(),
    ];
    for campus in &report.campuses {
        lines.push(format!(
            "- {}: {}/{} courses parsed across {} subject pages",
            campus.label,
            campus.entries.len(),
            campus.relevant_course_count,
            campus.parsed_subject_pages.len()
        ));
        if campus.used_index_snapshot_fallback || campus.subject_snapshot_fallback_count > 0 {
            lines.push(format!(
                "  - Used cached snapshot fallback: {}, {} subject page(s)",
                if campus.used_index_snapshot_fallback {
                    "index"
                } else {
                    "no index"
                },
                campus.subject_snapshot_fallback_count
            ));
        }
        if !campus.missing_subject_codes.is_empty() {
            lines.push(format!(
                "  - Missing subject pages: {}",
                campus
                    .missing_subject_codes
                    .iter()
                    .take(30)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !campus.invalid_planner_subject_codes.is_empty() {
            lines.push(format!(
                "  - Ignored invalid planner subject tokens: {}",
                campus
                    .invalid_planner_subject_codes
                    .iter()
                    .take(20)
                    .map(|entry| entry.subject_code.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    lines.push(String::new());
    fs::write(markdown_path, lines.join("\n") + "\n").map_err(|error| {
        format!(
            "could not write UW catalog summary {}: {error}",
            markdown_path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> CampusConfig {
        CAMPUSES[0]
    }

    #[test]
    fn parses_heading_and_credit_forms_deterministically() {
        let course = parse_course_heading("CSE 121 Introduction to Programming I (5)").unwrap();
        assert_eq!(course.code, "CSE 121");
        assert_eq!(course.title, "Introduction to Programming I");
        assert_eq!(course.credit_label.as_deref(), Some("5"));
        assert_eq!(course.credit_value.as_deref(), Some("5"));
        let variable = parse_course_heading("MATH 124 Calculus I (5) NW").unwrap();
        assert_eq!(variable.credit_label.as_deref(), Some("5"));
        assert_eq!(
            parse_course_heading("CSE 142 Computer Programming (4-5)")
                .unwrap()
                .credit_value,
            None
        );
        assert_eq!(
            parse_course_heading("ART 101 Studio (*)(5)")
                .unwrap()
                .credit_label
                .as_deref(),
            Some("5")
        );
    }

    #[test]
    fn extracts_official_requirement_notes_without_promoting_graph_edges() {
        let notes = extract_requirement_notes(
            "Prerequisite: MATH 124 or MATH 125. Recommended: prior programming. Corequisite: CSE 121. Offered: Autumn",
        );
        assert_eq!(
            notes.prerequisite_notes[0],
            "Official UW prerequisite text: MATH 124 or MATH 125."
        );
        assert_eq!(
            notes.prerequisite_notes[1],
            "prerequisite text is preserved as a note until a parser can safely normalize AND/OR/minimum-grade semantics into graph prerequisites."
        );
        assert_eq!(
            notes.prerequisite_notes[2],
            "Official UW recommended-preparation text: prior programming. Corequisite: CSE 121."
        );
        assert_eq!(
            notes.corequisite_notes[0],
            "Official UW corequisite text: CSE 121."
        );
    }

    #[test]
    fn discovers_subject_links_and_parses_only_requested_course_rows() {
        let index = r#"<a href="math.html">Mathematics (MATH)</a><a href="cse.html">Computer Science (CSE)</a>"#;
        let subjects =
            build_subject_url_map(index, "https://www.washington.edu/students/crscat/").unwrap();
        assert_eq!(
            subjects["MATH"].url,
            "https://www.washington.edu/students/crscat/math.html"
        );
        let html = r#"<a name="cse121"><p><b>CSE 121 Introduction to Programming (5)</b><br/>Prerequisite: none. Offered: Autumn<br/><a href="detail">View course details</a></p><a name="cse142"><p><b>CSE 142 Computer Programming (4-5)</b><br/>Description.</p>"#;
        let wanted = BTreeSet::from(["CSE 121".to_string()]);
        let entries = parse_subject_page(html, config(), &subjects["CSE"], &wanted).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].code, "CSE 121");
        assert_eq!(entries[0].credit_value, Some(serde_json::Number::from(5)));
        assert!(
            serde_json::to_string(&entries[0])
                .unwrap()
                .contains("\"creditValue\":5,")
        );
        assert_eq!(
            entries[0].source_links[0].url,
            "https://www.washington.edu/students/crscat/cse.html#cse121"
        );
    }

    #[test]
    fn subject_filtering_classifies_invalid_planner_labels() {
        let known = HashSet::from(["CSE".to_string(), "MATH".to_string()]);
        assert!(!is_likely_invalid_planner_subject("CSE", &known));
        assert!(is_likely_invalid_planner_subject("COURSE ENGL", &known));
        assert!(!is_likely_invalid_planner_subject("ASTR", &known));
    }
}
