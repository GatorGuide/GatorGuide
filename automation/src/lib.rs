use chrono::{SecondsFormat, Utc};
use reqwest::blocking::{Client, Response};
use scraper::{Html, Selector};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::net::IpAddr;
use std::time::Duration;
use url::Url;

const MAX_PAGE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_CANDIDATES_PER_PAGE: usize = 250;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_REDIRECTS: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpportunityCategory {
    Scholarships,
    Internships,
}

impl OpportunityCategory {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "scholarship" | "scholarships" => Ok(Self::Scholarships),
            "internship" | "internships" => Ok(Self::Internships),
            _ => Err(format!(
                "unsupported opportunity category {value:?}; expected scholarships or internships"
            )),
        }
    }

    fn catalog_type(self) -> &'static str {
        match self {
            Self::Scholarships => "scholarship",
            Self::Internships => "internship",
        }
    }

    fn matches_text(self, value: &str) -> bool {
        let text = value.to_ascii_lowercase();
        match self {
            Self::Scholarships => ["scholarship", "fellowship", "grant", "bursary", "award"]
                .iter()
                .any(|term| text.contains(term)),
            Self::Internships => [
                "internship",
                "internships",
                "intern ",
                "intern-",
                "intern_",
                "research experience",
                "undergraduate research",
            ]
            .iter()
            .any(|term| text.contains(term)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDefinition {
    pub opportunity_ids: Vec<String>,
    pub titles: Vec<String>,
    pub source_labels: Vec<String>,
    pub url: String,
}

#[derive(Debug, Serialize)]
pub struct AutomationReport {
    pub schema_version: u8,
    pub category: OpportunityCategory,
    pub generated_at_utc: String,
    pub publication_policy: &'static str,
    pub source_count: usize,
    pub sources: Vec<SourceReport>,
}

#[derive(Debug, Serialize)]
pub struct SourceReport {
    pub opportunity_ids: Vec<String>,
    pub titles: Vec<String>,
    pub source_labels: Vec<String>,
    pub source_url: String,
    pub state: String,
    pub http_status: Option<u16>,
    pub content_type: Option<String>,
    pub page_title: Option<String>,
    pub page_description: Option<String>,
    pub page_modified: Option<String>,
    pub structured_candidates: Vec<StructuredCandidate>,
    pub candidate_links: Vec<CandidateLink>,
    pub note: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct StructuredCandidate {
    pub title: String,
    pub url: Option<String>,
    pub description: Option<String>,
    pub deadline: Option<String>,
    pub schema_types: Vec<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct CandidateLink {
    pub title: String,
    pub url: String,
}

pub fn collect_sources(
    catalog: &Value,
    category: OpportunityCategory,
) -> Result<Vec<SourceDefinition>, String> {
    let entries = catalog
        .as_array()
        .or_else(|| catalog.get("opportunities").and_then(Value::as_array))
        .ok_or_else(|| {
            "opportunity catalog must be a JSON array or contain an opportunities array".to_string()
        })?;

    let mut by_url: BTreeMap<String, SourceDefinition> = BTreeMap::new();
    for (index, entry) in entries.iter().enumerate() {
        if entry.get("type").and_then(Value::as_str) != Some(category.catalog_type()) {
            continue;
        }

        let id = nonempty_string(entry.get("opportunityId")).ok_or_else(|| {
            format!(
                "{} entry at index {index} is missing opportunityId",
                category.catalog_type()
            )
        })?;
        let title = nonempty_string(entry.get("title"))
            .ok_or_else(|| format!("{} entry {id:?} is missing title", category.catalog_type()))?;
        let source_url = entry
            .get("source")
            .and_then(|source| nonempty_string(source.get("sourceUrl")))
            .or_else(|| nonempty_string(entry.get("externalUrl")))
            .ok_or_else(|| {
                format!(
                    "{} entry {id:?} is missing a source URL",
                    category.catalog_type()
                )
            })?;
        let parsed = validate_public_https_url(&source_url).map_err(|reason| {
            format!(
                "{} entry {id:?} has invalid source URL: {reason}",
                category.catalog_type()
            )
        })?;
        let canonical_url = parsed.to_string();
        let source_label = entry
            .get("source")
            .and_then(|source| nonempty_string(source.get("sourceLabel")))
            .unwrap_or_else(|| title.clone());
        let source = by_url
            .entry(canonical_url.clone())
            .or_insert_with(|| SourceDefinition {
                opportunity_ids: Vec::new(),
                titles: Vec::new(),
                source_labels: Vec::new(),
                url: canonical_url,
            });
        push_unique(&mut source.opportunity_ids, id);
        push_unique(&mut source.titles, title);
        push_unique(&mut source.source_labels, source_label);
    }

    let sources: Vec<_> = by_url.into_values().collect();
    if sources.is_empty() {
        return Err(format!(
            "no {} entries with supported sources were found",
            category.catalog_type()
        ));
    }
    Ok(sources)
}

pub fn build_report(
    catalog: &Value,
    category: OpportunityCategory,
    client: &Client,
) -> Result<AutomationReport, String> {
    let sources = collect_sources(catalog, category)?;
    let results = sources
        .iter()
        .map(|source| inspect_source(source, category, client))
        .collect();
    Ok(AutomationReport {
        schema_version: 1,
        category,
        generated_at_utc: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        publication_policy: "review_only; this report never modifies the public opportunity catalog",
        source_count: sources.len(),
        sources: results,
    })
}

pub fn build_http_client() -> Result<Client, String> {
    let redirect_policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("redirect limit exceeded");
        }
        let previous = attempt.previous().last();
        let next = attempt.url();
        let same_site = previous
            .and_then(Url::host_str)
            .zip(next.host_str())
            .is_some_and(|(old_host, new_host)| host_family(old_host) == host_family(new_host));
        if next.scheme() == "https"
            && next.username().is_empty()
            && next.password().is_none()
            && same_site
        {
            attempt.follow()
        } else {
            attempt.stop()
        }
    });

    Client::builder()
        .user_agent("GatorGuideDeterministicOpportunityReview/1.0 (+https://github.com/MarsLuay/GatorGuide)")
        .timeout(REQUEST_TIMEOUT)
        .redirect(redirect_policy)
        .build()
        .map_err(|error| format!("failed to configure HTTP client: {error}"))
}

fn inspect_source(
    source: &SourceDefinition,
    category: OpportunityCategory,
    client: &Client,
) -> SourceReport {
    let base = SourceReport {
        opportunity_ids: source.opportunity_ids.clone(),
        titles: source.titles.clone(),
        source_labels: source.source_labels.clone(),
        source_url: source.url.clone(),
        state: String::new(),
        http_status: None,
        content_type: None,
        page_title: None,
        page_description: None,
        page_modified: None,
        structured_candidates: Vec::new(),
        candidate_links: Vec::new(),
        note: None,
    };

    match fetch_page(client, &source.url) {
        Ok((response, content_type, bytes)) => {
            let status = response.status().as_u16();
            let mut result = base;
            result.http_status = Some(status);
            result.content_type = Some(content_type.clone());
            if !response.status().is_success() {
                result.state = "http_error".to_string();
                result.note = Some(format!("source returned HTTP {status}"));
                return result;
            }
            if !is_html_content_type(&content_type) {
                result.state = "unsupported_content_type".to_string();
                result.note = Some("only HTML and XHTML source pages are parsed".to_string());
                return result;
            }
            let html = String::from_utf8_lossy(&bytes);
            let findings = extract_page_findings(&html, &source.url, category);
            result.state = "checked".to_string();
            result.page_title = findings.page_title;
            result.page_description = findings.page_description;
            result.page_modified = findings.page_modified;
            result.structured_candidates = findings.structured_candidates;
            result.candidate_links = findings.candidate_links;
            result
        }
        Err(error) => SourceReport {
            state: "fetch_error".to_string(),
            note: Some(error),
            ..base
        },
    }
}

fn fetch_page(client: &Client, source_url: &str) -> Result<(Response, String, Vec<u8>), String> {
    let response = client
        .get(source_url)
        .send()
        .map_err(|error| format!("request failed: {error}"))?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PAGE_BYTES)
    {
        return Err(format!(
            "response exceeds the {MAX_PAGE_BYTES}-byte size limit"
        ));
    }
    let mut limited = response.take(MAX_PAGE_BYTES + 1);
    let mut bytes = Vec::new();
    limited
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read response body: {error}"))?;
    if bytes.len() as u64 > MAX_PAGE_BYTES {
        return Err(format!(
            "response exceeds the {MAX_PAGE_BYTES}-byte size limit"
        ));
    }
    let response = limited.into_inner();
    Ok((response, content_type, bytes))
}

#[derive(Default)]
struct PageFindings {
    page_title: Option<String>,
    page_description: Option<String>,
    page_modified: Option<String>,
    structured_candidates: Vec<StructuredCandidate>,
    candidate_links: Vec<CandidateLink>,
}

fn extract_page_findings(
    html: &str,
    source_url: &str,
    category: OpportunityCategory,
) -> PageFindings {
    let document = Html::parse_document(html);
    let source = Url::parse(source_url).ok();
    let mut findings = PageFindings {
        page_title: select_text(&document, "title").and_then(nonempty),
        page_description: meta_content(
            &document,
            &[
                "meta[property='og:description']",
                "meta[name='description']",
            ],
        ),
        page_modified: meta_content(
            &document,
            &[
                "meta[property='article:modified_time']",
                "meta[name='last-modified']",
                "meta[name='dateModified']",
            ],
        ),
        ..PageFindings::default()
    };
    if let Some(og_title) = meta_content(&document, &["meta[property='og:title']"]) {
        findings.page_title = Some(og_title);
    }

    let mut structured = BTreeSet::new();
    if let Ok(selector) = Selector::parse("script[type='application/ld+json']") {
        for script in document.select(&selector) {
            let script_text = script.text().collect::<String>();
            if let Ok(value) = serde_json::from_str::<Value>(&script_text) {
                collect_structured_candidates(&value, category, source.as_ref(), &mut structured);
            }
        }
    }
    findings.structured_candidates = structured.into_iter().collect();

    let mut links = BTreeSet::new();
    if let (Some(source), Ok(selector)) = (source.as_ref(), Selector::parse("a[href]")) {
        for anchor in document.select(&selector) {
            let title = normalize_space(&anchor.text().collect::<Vec<_>>().join(" "));
            let Some(href) = anchor.value().attr("href") else {
                continue;
            };
            if title.len() < 3 || !category.matches_text(&format!("{title} {href}")) {
                continue;
            }
            let Ok(url) = source.join(href) else {
                continue;
            };
            if validate_public_https_url(url.as_str()).is_err() || url == *source {
                continue;
            }
            if links.len() < MAX_CANDIDATES_PER_PAGE {
                links.insert(CandidateLink {
                    title,
                    url: url.to_string(),
                });
            }
        }
    }
    findings.candidate_links = links.into_iter().collect();
    findings
}

fn collect_structured_candidates(
    value: &Value,
    category: OpportunityCategory,
    source_url: Option<&Url>,
    output: &mut BTreeSet<StructuredCandidate>,
) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_structured_candidates(value, category, source_url, output);
            }
        }
        Value::Object(object) => {
            let types = json_string_values(object.get("@type"));
            let title = nonempty_string(object.get("name"));
            let description = nonempty_string(object.get("description"));
            let type_text = types.join(" ");
            let employment_type = json_string_values(object.get("employmentType")).join(" ");
            let internship_job_posting = category == OpportunityCategory::Internships
                && type_text.to_ascii_lowercase().contains("jobposting")
                && employment_type.to_ascii_lowercase().contains("intern");
            let relevant = category.matches_text(&type_text)
                || internship_job_posting
                || title
                    .as_deref()
                    .is_some_and(|text| category.matches_text(text))
                || description
                    .as_deref()
                    .is_some_and(|text| category.matches_text(text));
            if relevant {
                if let Some(title) = title {
                    let url = nonempty_string(object.get("url"))
                        .and_then(|value| resolve_public_url(value, source_url));
                    let deadline = [
                        "applicationDeadline",
                        "validThrough",
                        "deadline",
                        "endDate",
                        "expires",
                    ]
                    .iter()
                    .find_map(|key| nonempty_string(object.get(*key)));
                    if output.len() < MAX_CANDIDATES_PER_PAGE {
                        output.insert(StructuredCandidate {
                            title,
                            url,
                            description,
                            deadline,
                            schema_types: types,
                        });
                    }
                }
            }
            for child in object.values() {
                collect_structured_candidates(child, category, source_url, output);
            }
        }
        _ => {}
    }
}

fn resolve_public_url(value: String, source_url: Option<&Url>) -> Option<String> {
    let parsed = match Url::parse(&value) {
        Ok(parsed) => parsed,
        Err(_) => source_url?.join(&value).ok()?,
    };
    validate_public_https_url(parsed.as_str()).ok()?;
    Some(parsed.to_string())
}

fn validate_public_https_url(value: &str) -> Result<Url, String> {
    let parsed = Url::parse(value).map_err(|error| error.to_string())?;
    if parsed.scheme() != "https" {
        return Err("URL must use HTTPS".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("URL must not contain embedded credentials".to_string());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "URL must include a host".to_string())?;
    if host.eq_ignore_ascii_case("localhost")
        || host.ends_with(".localhost")
        || host.ends_with(".local")
    {
        return Err("local hostnames are not allowed".to_string());
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        let public = match ip {
            IpAddr::V4(ip) => {
                !(ip.is_private()
                    || ip.is_loopback()
                    || ip.is_link_local()
                    || ip.is_unspecified()
                    || ip.is_broadcast())
            }
            IpAddr::V6(ip) => {
                !(ip.is_loopback()
                    || ip.is_unique_local()
                    || ip.is_unicast_link_local()
                    || ip.is_unspecified())
            }
        };
        if !public {
            return Err("non-public IP addresses are not allowed".to_string());
        }
    }
    Ok(parsed)
}

fn host_family(host: &str) -> &str {
    host.strip_prefix("www.").unwrap_or(host)
}

fn is_html_content_type(content_type: &str) -> bool {
    content_type.contains("text/html") || content_type.contains("application/xhtml+xml")
}

fn select_text(document: &Html, selector_text: &str) -> Option<String> {
    let selector = Selector::parse(selector_text).ok()?;
    document
        .select(&selector)
        .next()
        .map(|element| normalize_space(&element.text().collect::<Vec<_>>().join(" ")))
        .and_then(nonempty)
}

fn meta_content(document: &Html, selectors: &[&str]) -> Option<String> {
    selectors.iter().find_map(|selector_text| {
        let selector = Selector::parse(selector_text).ok()?;
        document
            .select(&selector)
            .next()
            .and_then(|element| element.value().attr("content"))
            .map(str::to_string)
            .and_then(nonempty)
    })
}

fn nonempty_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::to_string)
        .and_then(nonempty)
}

fn nonempty(value: String) -> Option<String> {
    let value = normalize_space(&value);
    (!value.is_empty()).then_some(value)
}

fn normalize_space(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn json_string_values(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(value)) => vec![value.clone()],
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
        values.sort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn source_selection_is_category_scoped_sorted_and_deduplicated() {
        let catalog = json!([
            {"type":"scholarship","opportunityId":"s-2","title":"Scholarship B","externalUrl":"https://example.org/apply","source":{"sourceUrl":"https://example.org/apply","sourceLabel":"Official"}},
            {"type":"internship","opportunityId":"i-1","title":"Internship A","externalUrl":"https://interns.example.org/"},
            {"type":"scholarship","opportunityId":"s-1","title":"Scholarship A","externalUrl":"https://example.org/apply","source":{"sourceUrl":"https://example.org/apply","sourceLabel":"Official"}}
        ]);
        let sources = collect_sources(&catalog, OpportunityCategory::Scholarships).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].opportunity_ids, vec!["s-1", "s-2"]);
        assert_eq!(sources[0].titles, vec!["Scholarship A", "Scholarship B"]);
    }

    #[test]
    fn invalid_source_urls_fail_closed() {
        for url in [
            "http://example.org/",
            "https://localhost/",
            "https://127.0.0.1/",
        ] {
            assert!(validate_public_https_url(url).is_err(), "{url}");
        }
        assert!(validate_public_https_url("https://example.org/path").is_ok());
    }

    #[test]
    fn scholarship_parser_extracts_structured_candidates_and_relevant_links() {
        let html = r#"
          <html><head>
            <title>Foundation Opportunities</title>
            <meta property="og:title" content="GRC Scholarships">
            <meta name="description" content="Scholarship opportunities">
            <script type="application/ld+json">
              {"@context":"https://schema.org","@type":"ItemList","itemListElement":[
                {"@type":"Scholarship","name":"STEM Scholarship","url":"/stem","description":"Scholarship for students","validThrough":"2027-04-30"}
              ]}
            </script>
          </head><body>
            <a href="/stem">STEM Scholarship</a>
            <a href="/scholarship/renewable">View details</a>
            <a href="/about">About the foundation</a>
          </body></html>
        "#;
        let findings = extract_page_findings(
            html,
            "https://example.org/",
            OpportunityCategory::Scholarships,
        );
        assert_eq!(findings.page_title.as_deref(), Some("GRC Scholarships"));
        assert_eq!(findings.structured_candidates.len(), 1);
        assert_eq!(findings.structured_candidates[0].title, "STEM Scholarship");
        assert_eq!(
            findings.structured_candidates[0].url.as_deref(),
            Some("https://example.org/stem")
        );
        assert_eq!(
            findings.structured_candidates[0].deadline.as_deref(),
            Some("2027-04-30")
        );
        assert_eq!(findings.candidate_links.len(), 2);
        assert!(
            findings
                .candidate_links
                .iter()
                .any(|candidate| candidate.title == "STEM Scholarship")
        );
        assert!(
            findings
                .candidate_links
                .iter()
                .any(|candidate| candidate.title == "View details")
        );
    }

    #[test]
    fn internship_parser_does_not_treat_unrelated_navigation_as_candidates() {
        let html = r#"
          <html><head><title>Internships</title>
            <script type="application/ld+json">
              {"@type":"https://schema.org/JobPosting","name":"Summer Research Program","employmentType":"Internship","url":"/summer"}
            </script>
          </head><body>
            <a href="/internships/summer">Summer internship program</a>
            <a href="/careers">Careers</a>
            <a href="javascript:alert(1)">Internship details</a>
          </body></html>
        "#;
        let findings = extract_page_findings(
            html,
            "https://agency.example/",
            OpportunityCategory::Internships,
        );
        assert_eq!(findings.candidate_links.len(), 1);
        assert_eq!(
            findings.candidate_links[0].url,
            "https://agency.example/internships/summer"
        );
        assert_eq!(findings.structured_candidates.len(), 1);
        assert_eq!(
            findings.structured_candidates[0].url.as_deref(),
            Some("https://agency.example/summer")
        );
    }

    #[test]
    fn source_catalog_shape_errors_are_clear() {
        assert!(
            collect_sources(&json!({}), OpportunityCategory::Internships)
                .unwrap_err()
                .contains("JSON array")
        );
        assert!(
            collect_sources(&json!([]), OpportunityCategory::Scholarships)
                .unwrap_err()
                .contains("no scholarship entries")
        );
    }

    #[test]
    fn current_catalog_supplies_reviewed_sources_for_both_dormant_workflows() {
        let catalog: Value =
            serde_json::from_str(include_str!("../../source/data/starter-opportunities.json"))
                .expect("the public opportunity catalog must remain valid JSON");
        let scholarships = collect_sources(&catalog, OpportunityCategory::Scholarships).unwrap();
        let internships = collect_sources(&catalog, OpportunityCategory::Internships).unwrap();
        assert_eq!(scholarships.len(), 18);
        assert_eq!(internships.len(), 19);
        assert!(
            scholarships
                .iter()
                .chain(internships.iter())
                .all(|source| { validate_public_https_url(&source.url).is_ok() })
        );
    }
}
