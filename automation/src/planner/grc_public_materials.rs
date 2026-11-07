use chrono::{SecondsFormat, Utc};
use regex::Regex;
use reqwest::blocking::Client;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use url::Url;

const CLASS_SCHEDULES_URL: &str =
    "https://www.greenriver.edu/students/academics/class-schedules-catalog/index.html";
const CATALOG_ARCHIVE_URL: &str =
    "https://www.greenriver.edu/students/academics/class-schedules-catalog/catalog-archive.html";
const USER_AGENT: &str = "GatorGuideTransferPlannerGrcDiscovery/1.0";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GrcSchedule {
    pub label: String,
    pub url: String,
    pub output_path: String,
    pub owner_id: String,
    pub source_page_url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GrcAnnualScheduleCandidate {
    pub label: String,
    pub url: String,
    pub source_page_url: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrcAnnualScheduleCandidatesInput {
    pub html: String,
    pub base_url: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrcAnnualScheduleFilterInput {
    pub annual_schedules: Vec<serde_json::Value>,
    pub current_catalog_year_label: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GrcCatalogEntry {
    pub label: String,
    pub url: String,
    pub source_page_url: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrcCatalogArchiveEntriesInput {
    pub html: String,
    pub base_url: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrcCurrentCatalogDetailsInput {
    pub html: String,
    pub catalog_root_url: String,
    pub expected_catalog_year_label: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GrcCurrentCatalog {
    pub label: String,
    pub root_url: String,
    pub source_page_url: String,
    pub course_descriptions_url: String,
    pub course_descriptions_expanded_url: String,
    pub catoid: Option<String>,
    pub navoid: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GrcDiscoveryPages {
    pub class_schedules_url: String,
    pub catalog_archive_url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GrcPublicMaterials {
    pub generated_at: String,
    pub used_snapshot_fallback: bool,
    pub snapshot_fallback_reason: Option<String>,
    pub discovery_pages: GrcDiscoveryPages,
    pub annual_schedules: Vec<GrcSchedule>,
    pub discovered_annual_schedule_count: usize,
    pub catalog_entries: Vec<GrcCatalogEntry>,
    pub current_catalog: GrcCurrentCatalog,
}

pub fn discover(
    repository_root: &Path,
    force_refresh: bool,
    allow_snapshot_fallback: bool,
    cache_only: bool,
) -> Result<GrcPublicMaterials, String> {
    let source_root = repository_root.join("source");
    let report_path = source_root
        .join(".tmp")
        .join("reports")
        .join("transfer-planner-grc-public-materials.json");
    let markdown_path = source_root
        .join(".tmp")
        .join("reports")
        .join("transfer-planner-grc-public-materials.md");
    fs::create_dir_all(report_path.parent().expect("report path has a parent"))
        .map_err(|error| format!("could not create GRC discovery report directory: {error}"))?;

    let cached = load_cached(&report_path, &source_root)?;
    if cached.is_some() && (!force_refresh || cache_only) {
        return Ok(cached.expect("checked above"));
    }
    if cache_only {
        return Err(format!(
            "Cached Green River public-materials discovery is required in no-download mode, but {} was not found. Run the normal planner refresh once to create it.",
            report_path.display()
        ));
    }

    match discover_live(&source_root) {
        Ok(materials) => {
            write_report(&materials, &report_path, &markdown_path)?;
            Ok(materials)
        }
        Err(error) if allow_snapshot_fallback => {
            let Some(mut fallback) = cached else {
                return Err(error);
            };
            fallback.generated_at = now_iso();
            fallback.used_snapshot_fallback = true;
            fallback.snapshot_fallback_reason = Some(error);
            write_report(&fallback, &report_path, &markdown_path)?;
            Ok(fallback)
        }
        Err(error) => Err(error),
    }
}

fn discover_live(source_root: &Path) -> Result<GrcPublicMaterials, String> {
    let client = build_client()?;
    let schedules_html = fetch_text(&client, CLASS_SCHEDULES_URL)?;
    let discovered_schedules =
        extract_annual_schedules(&schedules_html, CLASS_SCHEDULES_URL, source_root)?;
    if discovered_schedules.is_empty() {
        return Err(format!(
            "No Green River annual schedule PDFs were found on {CLASS_SCHEDULES_URL}"
        ));
    }

    let archive_html = fetch_text(&client, CATALOG_ARCHIVE_URL)?;
    let catalog_entries = parse_catalog_archive_entries(&archive_html, CATALOG_ARCHIVE_URL)?;
    if catalog_entries.is_empty() {
        return Err(format!(
            "No Green River catalog archive entries were found on {CATALOG_ARCHIVE_URL}"
        ));
    }

    let current_entry = &catalog_entries[0];
    let catalog_html = fetch_text(&client, &current_entry.url)?;
    let current_catalog =
        parse_current_catalog_details(&catalog_html, &current_entry.url, &current_entry.label)?;
    let annual_schedules =
        filter_relevant_annual_schedules(discovered_schedules.clone(), &current_catalog.label);

    Ok(GrcPublicMaterials {
        generated_at: now_iso(),
        used_snapshot_fallback: false,
        snapshot_fallback_reason: None,
        discovery_pages: GrcDiscoveryPages {
            class_schedules_url: CLASS_SCHEDULES_URL.to_string(),
            catalog_archive_url: CATALOG_ARCHIVE_URL.to_string(),
        },
        annual_schedules,
        discovered_annual_schedule_count: discovered_schedules.len(),
        catalog_entries,
        current_catalog,
    })
}

fn build_client() -> Result<Client, String> {
    let redirect_policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("redirect limit exceeded");
        }
        if attempt.url().scheme() != "https"
            || !is_green_river_host(attempt.url().host_str().unwrap_or_default())
        {
            return attempt.stop();
        }
        attempt.follow()
    });
    Client::builder()
        .user_agent(USER_AGENT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(redirect_policy)
        .build()
        .map_err(|error| format!("could not configure GRC source client: {error}"))
}

fn fetch_text(client: &Client, url: &str) -> Result<String, String> {
    let response = client
        .get(url)
        .send()
        .map_err(|error| format!("GRC public materials fetch failed for {url}: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "GRC public materials fetch failed for {url}: HTTP {}",
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES)
    {
        return Err(format!(
            "GRC public materials response exceeded {MAX_RESPONSE_BYTES} bytes: {url}"
        ));
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            format!("could not read GRC public materials response from {url}: {error}")
        })?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(format!(
            "GRC public materials response exceeded {MAX_RESPONSE_BYTES} bytes: {url}"
        ));
    }
    String::from_utf8(bytes)
        .map_err(|error| format!("GRC public materials response was not UTF-8 ({url}): {error}"))
}

fn extract_annual_schedules(
    html: &str,
    base_url: &str,
    source_root: &Path,
) -> Result<Vec<GrcSchedule>, String> {
    Ok(parse_annual_schedule_candidates(html, base_url)?
        .into_iter()
        .map(|candidate| GrcSchedule {
            output_path: schedule_snapshot_path(source_root, &candidate.label)
                .display()
                .to_string(),
            owner_id: format!("grc-annual-schedule-{}", slugify_year(&candidate.label)),
            label: candidate.label,
            url: candidate.url,
            source_page_url: candidate.source_page_url,
        })
        .collect())
}

pub fn parse_annual_schedule_candidates(
    html: &str,
    base_url: &str,
) -> Result<Vec<GrcAnnualScheduleCandidate>, String> {
    let base =
        Url::parse(base_url).map_err(|error| format!("invalid GRC schedule page URL: {error}"))?;
    let anchors = anchors(html)?;
    let mut by_label = std::collections::BTreeMap::<String, GrcAnnualScheduleCandidate>::new();
    for (href, text) in anchors {
        let source_text = format!("{text} {href}");
        let Some(label) = find_academic_year(&source_text) else {
            continue;
        };
        if !source_text.to_ascii_lowercase().contains("annual schedule") || !is_pdf_url(&href) {
            continue;
        }
        let Some(url) = to_absolute_url(&base, &href) else {
            continue;
        };
        by_label.insert(
            label.clone(),
            GrcAnnualScheduleCandidate {
                label,
                url,
                source_page_url: base_url.to_string(),
            },
        );
    }
    let mut schedules: Vec<_> = by_label.into_values().collect();
    schedules.sort_by(|left, right| compare_academic_year_labels(&left.label, &right.label));
    Ok(schedules)
}

pub fn parse_catalog_archive_entries(
    html: &str,
    base_url: &str,
) -> Result<Vec<GrcCatalogEntry>, String> {
    extract_catalog_archive_entries(html, base_url)
}

fn extract_catalog_archive_entries(
    html: &str,
    base_url: &str,
) -> Result<Vec<GrcCatalogEntry>, String> {
    let base = Url::parse(base_url)
        .map_err(|error| format!("invalid GRC catalog archive URL: {error}"))?;
    let anchors = anchors(html)?;
    let mut by_label = std::collections::BTreeMap::<String, GrcCatalogEntry>::new();
    for (href, text) in anchors {
        let Some(label) = find_catalog_year(&text) else {
            continue;
        };
        let Some(url) = to_absolute_url(&base, &href) else {
            continue;
        };
        by_label.insert(
            label.clone(),
            GrcCatalogEntry {
                label,
                url,
                source_page_url: base_url.to_string(),
            },
        );
    }
    let mut entries: Vec<_> = by_label.into_values().collect();
    entries.sort_by(|left, right| compare_academic_year_labels(&right.label, &left.label));
    Ok(entries)
}

pub fn parse_current_catalog_details(
    html: &str,
    catalog_root_url: &str,
    expected_catalog_year_label: &str,
) -> Result<GrcCurrentCatalog, String> {
    extract_current_catalog_details(html, catalog_root_url, expected_catalog_year_label)
}

fn extract_current_catalog_details(
    html: &str,
    catalog_root_url: &str,
    expected_catalog_year_label: &str,
) -> Result<GrcCurrentCatalog, String> {
    let base = Url::parse(catalog_root_url)
        .map_err(|error| format!("invalid GRC catalog root URL: {error}"))?;
    let course_descriptions_url = anchors(html)?
        .into_iter()
        .find(|(_, text)| text.to_ascii_lowercase().contains("course descriptions"))
        .and_then(|(href, _)| to_absolute_url(&base, &href))
        .ok_or_else(|| format!("Could not find the current Green River course descriptions link on {catalog_root_url}"))?;

    let mut source_url = Url::parse(&course_descriptions_url)
        .map_err(|error| format!("invalid GRC course descriptions URL: {error}"))?;
    let catoid = source_url
        .query_pairs()
        .find(|(key, _)| key == "catoid")
        .map(|(_, value)| value.into_owned());
    let navoid = source_url
        .query_pairs()
        .find(|(key, _)| key == "navoid")
        .map(|(_, value)| value.into_owned());
    replace_query_param(&mut source_url, "expand", None);
    replace_query_param(&mut source_url, "print", None);

    let mut expanded_url = source_url.clone();
    replace_query_param(&mut expanded_url, "expand", Some("1"));
    if !expanded_url.query_pairs().any(|(key, _)| key == "print") {
        replace_query_param(&mut expanded_url, "print", Some(""));
    }

    Ok(GrcCurrentCatalog {
        label: expected_catalog_year_label.to_string(),
        root_url: catalog_root_url.to_string(),
        source_page_url: catalog_root_url.to_string(),
        course_descriptions_url: source_url.into(),
        course_descriptions_expanded_url: expanded_url.into(),
        catoid,
        navoid,
    })
}

fn replace_query_param(url: &mut Url, name: &str, value: Option<&str>) {
    let retained: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| key != name)
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    url.set_query(None);
    let mut pairs = url.query_pairs_mut();
    pairs.extend_pairs(retained);
    if let Some(value) = value {
        pairs.append_pair(name, value);
    }
}

pub fn detect_catalog_page_numbers(html: &str) -> Vec<usize> {
    let page_pattern = Regex::new(r"(?i)filter(?:%5B|\[)cpage(?:%5D|\])=(\d+)")
        .expect("catalog page pattern is valid");
    let mut page_numbers = BTreeSet::from([1]);
    for capture in page_pattern.captures_iter(html) {
        if let Ok(page_number) = capture[1].parse::<usize>() {
            page_numbers.insert(page_number);
        }
    }
    if let Some(max_page_number) = page_numbers.last().copied().filter(|page| *page > 1) {
        page_numbers.extend(1..=max_page_number);
    }
    page_numbers.into_iter().collect()
}

pub fn build_paged_course_descriptions_url(
    course_descriptions_url: &str,
    page_number: usize,
) -> Result<String, String> {
    let mut url = Url::parse(course_descriptions_url)
        .map_err(|error| format!("invalid GRC course descriptions URL: {error}"))?;
    replace_query_param(&mut url, "filter[item_type]", Some("3"));
    replace_query_param(&mut url, "filter[only_active]", Some("1"));
    replace_query_param(&mut url, "filter[3]", Some("1"));
    if page_number > 1 {
        replace_query_param(&mut url, "filter[cpage]", Some(&page_number.to_string()));
    } else {
        replace_query_param(&mut url, "filter[cpage]", None);
    }
    replace_query_param(&mut url, "expand", Some("1"));
    if !url.query_pairs().any(|(key, _)| key == "print") {
        replace_query_param(&mut url, "print", Some(""));
    }
    Ok(url.into())
}

fn filter_relevant_annual_schedules(
    annual_schedules: Vec<GrcSchedule>,
    current_catalog_year_label: &str,
) -> Vec<GrcSchedule> {
    let Some(current_start) = get_academic_year_start(current_catalog_year_label) else {
        return annual_schedules;
    };
    let minimum_start = current_start.saturating_sub(1);
    annual_schedules
        .into_iter()
        .filter(|schedule| {
            get_academic_year_start(&schedule.label).is_some_and(|year| year >= minimum_start)
        })
        .collect()
}

pub fn filter_relevant_annual_schedule_values(
    annual_schedules: Vec<serde_json::Value>,
    current_catalog_year_label: &str,
) -> Vec<serde_json::Value> {
    let Some(current_start) = get_academic_year_start(current_catalog_year_label) else {
        return annual_schedules;
    };
    let minimum_start = current_start.saturating_sub(1);
    annual_schedules
        .into_iter()
        .filter(|schedule| {
            schedule
                .get("label")
                .and_then(serde_json::Value::as_str)
                .and_then(get_academic_year_start)
                .is_some_and(|year| year >= minimum_start)
        })
        .collect()
}

fn load_cached(path: &Path, source_root: &Path) -> Result<Option<GrcPublicMaterials>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let contents = fs::read_to_string(path).map_err(|error| {
        format!(
            "could not read cached GRC discovery {}: {error}",
            path.display()
        )
    })?;
    let mut materials: GrcPublicMaterials = serde_json::from_str(&contents).map_err(|error| {
        format!(
            "cached GRC discovery {} is invalid JSON: {error}",
            path.display()
        )
    })?;
    for schedule in &mut materials.annual_schedules {
        schedule.output_path = schedule_snapshot_path(source_root, &schedule.label)
            .display()
            .to_string();
    }
    Ok(Some(materials))
}

fn write_report(
    materials: &GrcPublicMaterials,
    json_path: &Path,
    markdown_path: &Path,
) -> Result<(), String> {
    let mut json = serde_json::to_string_pretty(materials)
        .map_err(|error| format!("could not serialize GRC discovery report: {error}"))?;
    json.push('\n');
    fs::write(json_path, json).map_err(|error| {
        format!(
            "could not write GRC discovery report {}: {error}",
            json_path.display()
        )
    })?;
    fs::write(markdown_path, build_markdown(materials)).map_err(|error| {
        format!(
            "could not write GRC discovery summary {}: {error}",
            markdown_path.display()
        )
    })
}

fn build_markdown(materials: &GrcPublicMaterials) -> String {
    let mut lines = vec![
        "# Green River Public Materials Discovery".to_string(),
        String::new(),
        format!("Generated: {}", materials.generated_at),
        format!(
            "Used cached fallback: {}",
            if materials.used_snapshot_fallback {
                "yes"
            } else {
                "no"
            }
        ),
    ];
    if let Some(reason) = &materials.snapshot_fallback_reason {
        lines.push(format!("Fallback reason: {reason}"));
    }
    lines.extend([
        String::new(),
        format!(
            "- Class schedules page: {}",
            materials.discovery_pages.class_schedules_url
        ),
        format!(
            "- Catalog archive page: {}",
            materials.discovery_pages.catalog_archive_url
        ),
        format!(
            "- Current catalog root: {}",
            materials.current_catalog.root_url
        ),
        format!(
            "- Current catalog year: {}",
            materials.current_catalog.label
        ),
        format!(
            "- Current course descriptions: {}",
            materials.current_catalog.course_descriptions_url
        ),
        String::new(),
        "## Annual Schedules".to_string(),
        String::new(),
    ]);
    lines.extend(
        materials
            .annual_schedules
            .iter()
            .map(|schedule| format!("- {}: {}", schedule.label, schedule.url)),
    );
    lines.extend([
        String::new(),
        "## Catalog Archive Entries".to_string(),
        String::new(),
    ]);
    lines.extend(
        materials
            .catalog_entries
            .iter()
            .map(|entry| format!("- {}: {}", entry.label, entry.url)),
    );
    lines.push(String::new());
    lines.join("\n")
}

fn anchors(html: &str) -> Result<Vec<(String, String)>, String> {
    let selector = Selector::parse("a[href]")
        .map_err(|error| format!("could not build HTML anchor selector: {error}"))?;
    let document = Html::parse_document(html);
    Ok(document
        .select(&selector)
        .filter_map(|anchor| {
            let href = anchor.value().attr("href")?.trim();
            if href.is_empty() {
                return None;
            }
            let text = normalize_whitespace(&anchor.text().collect::<String>());
            Some((href.to_string(), text))
        })
        .collect())
}

fn to_absolute_url(base: &Url, href: &str) -> Option<String> {
    let parsed = base.join(href.trim()).ok()?;
    if parsed.scheme() != "https" || !is_green_river_host(parsed.host_str().unwrap_or_default()) {
        return None;
    }
    Some(parsed.into())
}

fn is_green_river_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "greenriver.edu" || host.ends_with(".greenriver.edu")
}

fn is_pdf_url(value: &str) -> bool {
    Url::parse(value)
        .ok()
        .is_some_and(|url| url.path().to_ascii_lowercase().ends_with(".pdf"))
        || value
            .to_ascii_lowercase()
            .split(['?', '#'])
            .next()
            .is_some_and(|path| path.ends_with(".pdf"))
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn find_academic_year(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    for start in 0..bytes.len().saturating_sub(8) {
        let candidate = &bytes[start..start + 9];
        if candidate.len() == 9
            && candidate[0..2] == *b"20"
            && candidate[2].is_ascii_digit()
            && candidate[3].is_ascii_digit()
            && candidate[4] == b'-'
            && candidate[5..7] == *b"20"
            && candidate[7].is_ascii_digit()
            && candidate[8].is_ascii_digit()
        {
            return Some(String::from_utf8_lossy(candidate).to_string());
        }
    }
    None
}

fn find_catalog_year(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let prefix = "green river college";
    let start = lower.find(prefix)? + prefix.len();
    let tail = text[start..].trim_start();
    let label = find_academic_year(tail)?;
    let index = tail.find(&label)? + label.len();
    let after_year = tail[index..].trim_start();
    after_year
        .get(..7)
        .filter(|next| next.eq_ignore_ascii_case("catalog"))
        .map(|_| label)
}

fn get_academic_year_start(label: &str) -> Option<i32> {
    let year = label.get(..4)?.parse::<i32>().ok()?;
    if label.as_bytes().get(4) != Some(&b'-') {
        return None;
    }
    Some(year)
}

fn compare_academic_year_labels(left: &str, right: &str) -> std::cmp::Ordering {
    get_academic_year_start(left)
        .zip(get_academic_year_start(right))
        .map(|(left_year, right_year)| left_year.cmp(&right_year))
        .filter(|order| !order.is_eq())
        .unwrap_or_else(|| left.cmp(right))
}

fn slugify_year(label: &str) -> String {
    label
        .chars()
        .filter(|char| char.is_ascii_digit())
        .fold(String::new(), |mut output, digit| {
            if !output.is_empty() && output.len() == 4 {
                output.push('-');
            }
            output.push(digit);
            output
        })
}

fn schedule_snapshot_path(source_root: &Path, label: &str) -> PathBuf {
    source_root
        .join(".tmp")
        .join("downloads")
        .join(format!("{label}-Annual-Schedule.pdf"))
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn catalog_pagination_parser_fills_gaps_and_sorts_encoded_links() {
        let html = r#"
            <a href="?filter%5Bcpage%5D=10">10</a>
            <a href="?filter[cpage]=12">12</a>
            <a href="?filter%5Bcpage%5D=10">duplicate</a>
        "#;
        assert_eq!(
            detect_catalog_page_numbers(html),
            (1..=12).collect::<Vec<_>>()
        );
        assert_eq!(detect_catalog_page_numbers("no pagination links"), vec![1]);
        assert_eq!(detect_catalog_page_numbers("?filter[cpage]=0"), vec![0, 1]);
    }

    #[test]
    fn annual_schedule_discovery_filters_and_sorts_pdf_links() {
        let html = r#"
            <a href="/students/schedules/2025-2026 Annual Schedule.pdf">2025-2026 Annual Schedule</a>
            <a href="https://catalog.greenriver.edu/schedule/2024-2025.pdf">2024-2025 Annual Schedule</a>
            <a href="/schedules/2025-2026.pdf">2025-2026 Catalog</a>
            <a href="https://evil.example/2026-2027%20Annual%20Schedule.pdf">2026-2027 Annual Schedule</a>
        "#;
        let root = PathBuf::from("C:/gator-guide/source");
        let schedules = extract_annual_schedules(html, CLASS_SCHEDULES_URL, &root).unwrap();
        assert_eq!(schedules.len(), 2);
        assert_eq!(schedules[0].label, "2024-2025");
        assert_eq!(schedules[1].label, "2025-2026");
        assert_eq!(schedules[1].owner_id, "grc-annual-schedule-2025-2026");
        assert!(
            schedules[1]
                .output_path
                .ends_with("2025-2026-Annual-Schedule.pdf")
        );
    }

    #[test]
    fn annual_schedule_value_filter_preserves_opaque_fields_and_invalid_year_fallback() {
        let schedules = vec![
            serde_json::json!({"label": "2020-2021", "metadata": 1}),
            serde_json::json!({"label": "2024-2025", "metadata": 2}),
            serde_json::json!({"label": "2025-2026", "metadata": 3}),
            serde_json::json!({"label": "2026-2027", "metadata": 4}),
            serde_json::json!({"metadata": 5}),
        ];
        let filtered = filter_relevant_annual_schedule_values(schedules.clone(), "2025-2026");
        assert_eq!(
            filtered,
            schedules[1..4].to_vec(),
            "the Rust adapter preserves the legacy cutoff and opaque fields"
        );
        assert_eq!(
            filter_relevant_annual_schedule_values(schedules.clone(), "catalog-year-unknown"),
            schedules
        );
    }

    #[test]
    fn archive_entries_are_current_first_and_require_the_catalog_title() {
        let html = r#"
            <a href="/2023-2024">Green River College 2023-2024 Catalog</a>
            <a href="/2025-2026">Green River College 2025-2026 Catalog</a>
            <a href="/2026-2027">Green River College 2026-2027 Course Schedule</a>
        "#;
        let entries = parse_catalog_archive_entries(html, CATALOG_ARCHIVE_URL).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].label, "2025-2026");
        assert_eq!(entries[0].url, "https://www.greenriver.edu/2025-2026");
    }

    #[test]
    fn current_catalog_urls_preserve_catalog_ids_and_add_expanded_query() {
        let html = r#"<a href="/course-descriptions?catoid=81&amp;navoid=3001&amp;expand=1&amp;print=">Course Descriptions</a>"#;
        let catalog =
            parse_current_catalog_details(html, "https://catalog.greenriver.edu/", "2025-2026")
                .unwrap();
        assert_eq!(catalog.catoid.as_deref(), Some("81"));
        assert_eq!(catalog.navoid.as_deref(), Some("3001"));
        assert_eq!(
            catalog.course_descriptions_url,
            "https://catalog.greenriver.edu/course-descriptions?catoid=81&navoid=3001"
        );
        assert_eq!(
            catalog.course_descriptions_expanded_url,
            "https://catalog.greenriver.edu/course-descriptions?catoid=81&navoid=3001&expand=1&print="
        );
    }

    #[test]
    fn paginated_catalog_url_sets_filters_and_omits_first_page_number() {
        let first = build_paged_course_descriptions_url(
            "https://catalog.greenriver.edu/course-descriptions?catoid=81&navoid=3001",
            1,
        )
        .unwrap();
        let second = build_paged_course_descriptions_url(
            "https://catalog.greenriver.edu/course-descriptions?catoid=81&navoid=3001",
            2,
        )
        .unwrap();
        let first_url = Url::parse(&first).unwrap();
        let second_url = Url::parse(&second).unwrap();
        assert!(
            !first_url
                .query_pairs()
                .any(|(key, _)| key == "filter[cpage]")
        );
        assert_eq!(
            second_url
                .query_pairs()
                .find(|(key, _)| key == "filter[cpage]")
                .unwrap()
                .1,
            "2"
        );
        assert_eq!(
            second_url
                .query_pairs()
                .find(|(key, _)| key == "filter[item_type]")
                .unwrap()
                .1,
            "3"
        );
    }

    #[test]
    fn schedules_are_filtered_to_current_and_recent_prior_year() {
        let source = |label: &str| GrcSchedule {
            label: label.into(),
            url: format!("https://www.greenriver.edu/{label}.pdf"),
            output_path: String::new(),
            owner_id: format!("grc-annual-schedule-{label}"),
            source_page_url: CLASS_SCHEDULES_URL.into(),
        };
        let filtered = filter_relevant_annual_schedules(
            vec![
                source("2023-2024"),
                source("2024-2025"),
                source("2025-2026"),
            ],
            "2025-2026",
        );
        assert_eq!(
            filtered
                .iter()
                .map(|entry| entry.label.as_str())
                .collect::<Vec<_>>(),
            ["2024-2025", "2025-2026"]
        );
    }
}
