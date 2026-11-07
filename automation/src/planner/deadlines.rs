use chrono::{DateTime, Datelike, Duration, NaiveDate, SecondsFormat, TimeZone, Utc};
use chrono_tz::America::Los_Angeles;
use regex::Regex;
use reqwest::blocking::Client;
use scraper::{Html, Selector};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::Duration as StdDuration;

const TIMEZONE: &str = "America/Los_Angeles";
const KIND: &str = "official_page";
const USER_AGENT: &str = "GatorGuideDeadlineRefresh/1.0";
const RECENT_PAST_WINDOW_DAYS: i64 = 14;
const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const SAME_HOST_DELAY: StdDuration = StdDuration::from_millis(1_500);

#[derive(Clone, Copy)]
struct Source {
    key: &'static str,
    url: &'static str,
    label: &'static str,
}

const GRC_REGISTRAR: Source = Source {
    key: "grc-registrar",
    url: "https://www.greenriver.edu/students/academics/office-of-the-registrar/index.html",
    label: "Green River Office of the Registrar Important Upcoming Dates",
};
const GRC_FINANCIAL_AID: Source = Source {
    key: "grc-financial-aid",
    url: "https://www.greenriver.edu/students/pay-for-college/financial-aid/financial-aid-deadlines.html",
    label: "Green River financial aid deadlines page",
};
const UW_SEATTLE_TRANSFER: Source = Source {
    key: "uw-seattle-transfer",
    url: "https://admit.washington.edu/apply/transfer/",
    label: "UW Office of Admissions transfer dates and deadlines",
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct TermInfo {
    term: String,
    year: i32,
}

#[derive(Clone, Copy)]
struct MonthDay {
    month: u32,
    day: u32,
}

#[derive(Clone)]
struct RegistrarSpec {
    pattern: &'static str,
    id: &'static str,
    title_prefix: &'static str,
    label: &'static str,
    deadline_type: &'static str,
    hour: u32,
    summary_kind: SummaryKind,
}

#[derive(Clone, Copy)]
enum SummaryKind {
    Registrar,
    Priority,
    Credits90,
    Credits60,
    Credits30,
    Credits0,
    NewStudent,
}

#[derive(Clone)]
struct RawDeadline {
    spec: RegistrarSpec,
    term: TermInfo,
    due_at: String,
}

pub fn refresh(repository_root: &Path) -> Result<DeadlineRefreshReport, String> {
    let source_root = repository_root.join("source");
    let opportunities_path = source_root.join("data/starter-opportunities.json");
    let report_json_path = source_root.join(".tmp/reports/deadline-refresh-report.json");
    let report_md_path = source_root.join(".tmp/reports/deadline-refresh-report.md");
    fs::create_dir_all(report_json_path.parent().expect("report path has a parent"))
        .map_err(|error| format!("could not create deadline report directory: {error}"))?;

    let client = build_client()?;
    let now_iso = now_iso();
    let registrar_html = fetch_html(&client, GRC_REGISTRAR)?;
    let registrar = parse_grc_registrar_deadlines(&registrar_html, &now_iso)?;
    std::thread::sleep(SAME_HOST_DELAY);
    let financial_html = fetch_html(&client, GRC_FINANCIAL_AID)?;
    let financial_aid = parse_grc_financial_aid_deadlines(&financial_html, &now_iso)?;
    let transfer_html = fetch_html(&client, UW_SEATTLE_TRANSFER)?;
    let uw_transfer = parse_uw_seattle_transfer_deadlines(&transfer_html, &now_iso)?;

    let source_reports = vec![
        source_report(GRC_REGISTRAR, registrar.len()),
        source_report(GRC_FINANCIAL_AID, financial_aid.len()),
        source_report(UW_SEATTLE_TRANSFER, uw_transfer.len()),
    ];
    let parsed_deadlines: Vec<Value> = financial_aid
        .into_iter()
        .chain(registrar)
        .chain(uw_transfer)
        .collect();
    let mut parsed_by_id = HashMap::<String, Value>::new();
    for item in &parsed_deadlines {
        let id = item
            .get("opportunityId")
            .and_then(Value::as_str)
            .ok_or("generated deadline has no opportunityId")?;
        if parsed_by_id.insert(id.to_string(), item.clone()).is_some() {
            return Err("Deadline refresh produced duplicate opportunity ids.".to_string());
        }
    }

    let contents = fs::read_to_string(&opportunities_path).map_err(|error| {
        format!(
            "could not read opportunity catalog {}: {error}",
            opportunities_path.display()
        )
    })?;
    let existing: Vec<Value> = serde_json::from_str(&contents).map_err(|error| {
        format!(
            "opportunity catalog {} is invalid JSON: {error}",
            opportunities_path.display()
        )
    })?;
    let existing_ids: std::collections::HashSet<String> = existing
        .iter()
        .filter_map(|item| item.get("opportunityId").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    let mut created = Vec::new();
    let mut updated = Vec::new();
    let mut unchanged = Vec::new();
    let mut next = Vec::with_capacity(existing.len() + parsed_deadlines.len());

    for item in existing {
        let id = item
            .get("opportunityId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(candidate) = parsed_by_id.get(id) else {
            next.push(item);
            continue;
        };
        match apply_opportunity_update(&item, candidate, &now_iso) {
            UpdateResult::Unchanged(value) => {
                unchanged.push(id.to_string());
                next.push(value);
            }
            UpdateResult::Updated(value) => {
                updated.push(id.to_string());
                next.push(value);
            }
        }
    }
    for candidate in parsed_deadlines {
        let id = candidate
            .get("opportunityId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !existing_ids.contains(&id) {
            created.push(id);
            next.push(candidate);
        }
    }

    let mut encoded = serde_json::to_string_pretty(&next)
        .map_err(|error| format!("could not encode opportunity catalog: {error}"))?;
    encoded.push('\n');
    atomic_write(&opportunities_path, encoded.as_bytes())?;

    let report = DeadlineRefreshReport {
        checked_at: now_iso,
        sources: source_reports,
        parsed_count: parsed_by_id.len(),
        created,
        updated,
        unchanged,
        output_path: opportunities_path.display().to_string(),
        report_path: report_json_path.display().to_string(),
    };
    write_report(&report, &report_json_path, &report_md_path)?;
    Ok(report)
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadlineRefreshReport {
    pub checked_at: String,
    pub sources: Vec<DeadlineSourceReport>,
    pub parsed_count: usize,
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub unchanged: Vec<String>,
    pub output_path: String,
    pub report_path: String,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadlineSourceReport {
    pub key: String,
    pub label: String,
    pub url: String,
    pub parsed_count: usize,
}

fn source_report(source: Source, parsed_count: usize) -> DeadlineSourceReport {
    DeadlineSourceReport {
        key: source.key.to_string(),
        label: source.label.to_string(),
        url: source.url.to_string(),
        parsed_count,
    }
}

fn build_client() -> Result<Client, String> {
    let redirect_policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("redirect limit exceeded");
        }
        let next = attempt.url();
        let host = next.host_str().unwrap_or_default().to_ascii_lowercase();
        if next.scheme() == "https"
            && (host == "greenriver.edu"
                || host.ends_with(".greenriver.edu")
                || host == "washington.edu"
                || host.ends_with(".washington.edu"))
        {
            attempt.follow()
        } else {
            attempt.stop()
        }
    });
    Client::builder()
        .user_agent(USER_AGENT)
        .timeout(StdDuration::from_secs(30))
        .redirect(redirect_policy)
        .build()
        .map_err(|error| format!("could not configure deadline source client: {error}"))
}

fn fetch_html(client: &Client, source: Source) -> Result<String, String> {
    let mut last_error = None;
    for attempt in 0..2 {
        match fetch_html_once(client, source) {
            Ok(html) => return Ok(html),
            Err(error) => {
                last_error = Some(error);
                if attempt == 0 {
                    std::thread::sleep(StdDuration::from_millis(500));
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| format!("deadline source fetch failed: {}", source.url)))
}

fn fetch_html_once(client: &Client, source: Source) -> Result<String, String> {
    let response = client
        .get(source.url)
        .send()
        .map_err(|error| format!("Fetch deadline source {} failed: {error}", source.key))?;
    if !response.status().is_success() {
        return Err(format!(
            "Fetch deadline source {} failed: HTTP {}",
            source.key,
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES)
    {
        return Err(format!(
            "Fetch deadline source {} exceeded {MAX_RESPONSE_BYTES} bytes",
            source.key
        ));
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read deadline source {}: {error}", source.key))?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(format!(
            "Fetch deadline source {} exceeded {MAX_RESPONSE_BYTES} bytes",
            source.key
        ));
    }
    String::from_utf8(bytes)
        .map_err(|error| format!("deadline source {} was not UTF-8: {error}", source.key))
}

fn extract_table_after(html: &str, heading_text: &str) -> Result<Vec<Vec<String>>, String> {
    let lower_html = html.to_ascii_lowercase();
    let lower_heading = heading_text.to_ascii_lowercase();
    let heading_index = lower_html
        .find(&lower_heading)
        .or_else(|| {
            let plain = heading_text.replace("&amp;", "&");
            lower_html.find(&plain.to_ascii_lowercase())
        })
        .ok_or_else(|| format!("Could not find heading: {heading_text}"))?;
    let table_start = lower_html[heading_index..]
        .find("<table")
        .map(|index| heading_index + index)
        .ok_or_else(|| format!("Could not find table after heading: {heading_text}"))?;
    let table_end = lower_html[table_start..]
        .find("</table>")
        .map(|index| table_start + index + "</table>".len())
        .ok_or_else(|| format!("Could not find table end after heading: {heading_text}"))?;
    let fragment = Html::parse_fragment(&html[table_start..table_end]);
    let row_selector = Selector::parse("tr").map_err(|error| error.to_string())?;
    let cell_selector = Selector::parse("th, td").map_err(|error| error.to_string())?;
    Ok(fragment
        .select(&row_selector)
        .map(|row| {
            row.select(&cell_selector)
                .map(|cell| normalize_whitespace(&cell.text().collect::<Vec<_>>().join(" ")))
                .collect::<Vec<_>>()
        })
        .filter(|cells| !cells.is_empty())
        .collect())
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_quarter_header(value: &str) -> Option<TermInfo> {
    let regex = Regex::new(r"(?i)\b(Winter|Spring|Summer|Fall)\s+(\d{4})\b").ok()?;
    let captures = regex.captures(value)?;
    Some(TermInfo {
        term: capitalize_term(captures.get(1)?.as_str()),
        year: captures.get(2)?.as_str().parse().ok()?,
    })
}

fn capitalize_term(value: &str) -> String {
    let mut chars = value.chars();
    chars
        .next()
        .map(|first| {
            first.to_uppercase().collect::<String>() + &chars.as_str().to_ascii_lowercase()
        })
        .unwrap_or_default()
}

fn month_number(value: &str) -> Option<u32> {
    match value.trim_end_matches('.').to_ascii_lowercase().as_str() {
        "jan" | "january" => Some(1),
        "feb" | "february" => Some(2),
        "mar" | "march" => Some(3),
        "apr" | "april" => Some(4),
        "may" => Some(5),
        "jun" | "june" => Some(6),
        "jul" | "july" => Some(7),
        "aug" | "august" => Some(8),
        "sep" | "sept" | "september" => Some(9),
        "oct" | "october" => Some(10),
        "nov" | "november" => Some(11),
        "dec" | "december" => Some(12),
        _ => None,
    }
}

fn parse_first_month_day(value: &str) -> Option<MonthDay> {
    let mut text = value.replace('\u{200b}', "").replace('\u{feff}', "");
    let glued_month_day = Regex::new(r"(?i)\b([A-Za-z]{3,9})(\d{1,2})\b").ok()?;
    text = glued_month_day.replace_all(&text, "$1 $2").into_owned();
    let text = normalize_whitespace(&text);
    if text.is_empty() || text.chars().all(|ch| ch == '-') || text.contains("---") {
        return None;
    }
    let regex = Regex::new(r"(?i)\b([A-Za-z]{3,9})\.?\s+(\d{1,2})\b").ok()?;
    let captures = regex.captures(&text)?;
    let month = month_number(captures.get(1)?.as_str())?;
    let day = captures.get(2)?.as_str().parse::<u32>().ok()?;
    if !(1..=31).contains(&day) {
        return None;
    }
    Some(MonthDay { month, day })
}

fn parse_explicit_date(value: &str) -> Option<(MonthDay, i32)> {
    let month_day = parse_first_month_day(value)?;
    let captures = Regex::new(r"\b(20\d{2})\b").ok()?.captures(value)?;
    Some((month_day, captures.get(1)?.as_str().parse().ok()?))
}

fn zoned_datetime_to_iso(year: i32, month: u32, day: u32, hour: u32) -> Result<String, String> {
    let date = NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|date| date.and_hms_opt(hour, 0, 0))
        .ok_or_else(|| format!("invalid local deadline date {year}-{month}-{day} {hour}:00"))?;
    let local = Los_Angeles
        .from_local_datetime(&date)
        .single()
        .ok_or_else(|| format!("ambiguous local deadline time {year}-{month}-{day} {hour}:00"))?;
    Ok(local
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn get_cutoff_iso(now_iso: &str) -> Result<String, String> {
    let now = DateTime::parse_from_rfc3339(now_iso)
        .map_err(|error| format!("invalid refresh timestamp {now_iso:?}: {error}"))?;
    Ok((now - Duration::days(RECENT_PAST_WINDOW_DAYS))
        .to_utc()
        .to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn format_term(term: &TermInfo) -> String {
    format!("{} {}", term.term, term.year)
}

fn format_term_group(terms: &[TermInfo]) -> String {
    if terms.len() == 1 {
        return format_term(&terms[0]);
    }
    let years: Vec<i32> = terms
        .iter()
        .map(|term| term.year)
        .fold(Vec::new(), |mut years, year| {
            if !years.contains(&year) {
                years.push(year);
            }
            years
        });
    if years.len() == 1 {
        format!(
            "{} {}",
            terms
                .iter()
                .map(|term| term.term.as_str())
                .collect::<Vec<_>>()
                .join("/"),
            years[0]
        )
    } else {
        terms.iter().map(format_term).collect::<Vec<_>>().join("/")
    }
}

fn slugify(value: &str) -> String {
    let expanded = value
        .to_ascii_lowercase()
        .replace('&', " and ")
        .replace('+', "plus")
        .replace('%', " percent ");
    let mut output = String::new();
    let mut dash = false;
    for character in expanded.chars() {
        if character.is_ascii_alphanumeric() {
            if dash && !output.is_empty() {
                output.push('-');
            }
            output.push(character);
            dash = false;
        } else {
            dash = true;
        }
    }
    output
}

fn term_sort_index(term: &str) -> usize {
    match term {
        "Winter" => 0,
        "Spring" => 1,
        "Summer" => 2,
        "Fall" => 3,
        _ => 4,
    }
}

fn sort_terms(terms: &mut [TermInfo]) {
    terms.sort_by(|left, right| {
        left.year
            .cmp(&right.year)
            .then_with(|| term_sort_index(&left.term).cmp(&term_sort_index(&right.term)))
    });
}

fn format_term_group_slug(terms: &[TermInfo], use_and_for_pair: bool) -> String {
    if terms.len() == 1 {
        return format!("{}-{}", slugify(&terms[0].term), terms[0].year);
    }
    let years: Vec<i32> = terms
        .iter()
        .map(|term| term.year)
        .fold(Vec::new(), |mut years, year| {
            if !years.contains(&year) {
                years.push(year);
            }
            years
        });
    let separator = if use_and_for_pair && terms.len() == 2 {
        "-and-"
    } else {
        "-"
    };
    if years.len() == 1 {
        format!(
            "{}-{}",
            terms
                .iter()
                .map(|term| slugify(&term.term))
                .collect::<Vec<_>>()
                .join(separator),
            years[0]
        )
    } else {
        terms
            .iter()
            .map(|term| format!("{}-{}", slugify(&term.term), term.year))
            .collect::<Vec<_>>()
            .join(separator)
    }
}

fn build_source(source: Source, now_iso: &str) -> Value {
    json!({
        "kind": KIND,
        "sourceUrl": source.url,
        "sourceLabel": source.label,
        "model": null,
        "fetchedAt": now_iso,
        "verifiedAt": now_iso,
    })
}

fn build_base_opportunity(
    opportunity_id: String,
    title: String,
    organization_name: &str,
    summary: String,
    external_url: &str,
    due_at: String,
    deadline_type: &str,
    deadline_label: &str,
    matching: Value,
    eligibility: Value,
    college: Value,
    source: Source,
    now_iso: &str,
    recurrence: Value,
) -> Value {
    json!({
        "schemaVersion": 1,
        "opportunityId": opportunity_id,
        "type": "college_deadline",
        "status": "active",
        "title": title,
        "organizationName": organization_name,
        "summary": summary,
        "externalUrl": external_url,
        "dueAt": due_at,
        "recurrence": recurrence,
        "deadline": {"type": deadline_type, "label": deadline_label},
        "matching": matching,
        "eligibility": eligibility,
        "requirements": {"needsRecommendations": false, "recommendationCountMin": 0, "essayCount": 0},
        "award": {"amountMin": null, "amountMax": null, "currency": "USD", "amountText": null, "renewable": null},
        "college": college,
        "source": build_source(source, now_iso),
        "createdAt": now_iso,
        "updatedAt": now_iso,
    })
}

fn default_recurrence() -> Value {
    json!({"isYearly": false, "month": null, "day": null, "timezone": TIMEZONE})
}
fn empty_matching() -> Value {
    json!({"financialAidTags": [], "suggestedMajors": [], "hasToBeMajor": false})
}
fn default_eligibility() -> Value {
    json!({"gpaMin": null, "residencyTypes": [], "transferOnly": false})
}
fn grc_college() -> Value {
    json!({"collegeId": null, "collegeName": "Green River College", "city": "Auburn", "state": "WA", "website": "https://www.greenriver.edu/"})
}
fn uw_college() -> Value {
    json!({"collegeId": null, "collegeName": "University of Washington Seattle", "city": "Seattle", "state": "WA", "website": "https://www.washington.edu/"})
}

fn registrar_specs() -> Vec<RegistrarSpec> {
    let rows = [
        (
            r"(?i)^Priority Enrollment",
            "green-river-priority-enrollment-for",
            "Priority Class Enrollment for ",
            "Priority enrollment opens",
            "priority",
            8,
            SummaryKind::Priority,
        ),
        (
            r"(?i)^Current/Returning Enrollment:\s*90\+\s*cr",
            "green-river-current-returning-enrollment-90plus-credits-for",
            "90+ Credits Class Enrollment for ",
            "Class enrollment opens",
            "priority",
            8,
            SummaryKind::Credits90,
        ),
        (
            r"(?i)^Current/Returning Enrollment:\s*60\s*-\s*89\s*cr",
            "green-river-current-returning-enrollment-60-89-credits-for",
            "60-89 Credits Class Enrollment for ",
            "Class enrollment opens",
            "priority",
            8,
            SummaryKind::Credits60,
        ),
        (
            r"(?i)^Current/Returning Enrollment:\s*30\s*-\s*59\s*cr",
            "green-river-current-returning-enrollment-30-59-credits-for",
            "30-59 Credits Class Enrollment for ",
            "Class enrollment opens",
            "priority",
            8,
            SummaryKind::Credits30,
        ),
        (
            r"(?i)^Current/Returning Enrollment:\s*0\s*-\s*29\s*cr",
            "green-river-current-returning-enrollment-0-29-credits-for",
            "0-29 Credits Class Enrollment for ",
            "Class enrollment opens",
            "priority",
            8,
            SummaryKind::Credits0,
        ),
        (
            r"(?i)^New Student Enrollment",
            "green-river-new-student-enrollment-for",
            "New Student Class Enrollment for ",
            "New student enrollment opens",
            "priority",
            9,
            SummaryKind::NewStudent,
        ),
        (
            r"(?i)^Tuition or 1st Payment Plan Due Date",
            "green-river-tuition-or-first-payment-plan-due-for",
            "Tuition or First Payment Plan Due for ",
            "Tuition due date",
            "final",
            9,
            SummaryKind::Registrar,
        ),
        (
            r"(?i)^Waitlist Closes",
            "green-river-waitlist-closes-for",
            "Waitlist Closes for ",
            "Waitlist closes",
            "final",
            9,
            SummaryKind::Registrar,
        ),
        (
            r"(?i)^Quarter Begins",
            "green-river-quarter-begins-for",
            "Quarter Begins for ",
            "Quarter begins",
            "final",
            9,
            SummaryKind::Registrar,
        ),
        (
            r"(?i)^Instructor Permission Required to Add Class",
            "green-river-instructor-permission-required-to-add-class-for",
            "Instructor Permission Required to Add Class for ",
            "Instructor permission required",
            "final",
            9,
            SummaryKind::Registrar,
        ),
        (
            r"(?i)^100% Refund Deadline",
            "green-river-100-percent-refund-deadline-for",
            "100% Refund Deadline for ",
            "100% refund deadline",
            "final",
            9,
            SummaryKind::Registrar,
        ),
        (
            r"(?i)^Census Date",
            "green-river-census-date-for",
            "Census Date for ",
            "Census date",
            "final",
            9,
            SummaryKind::Registrar,
        ),
        (
            r"(?i)^Last day to withdraw, not posted on transcript",
            "green-river-last-day-to-withdraw-without-transcript-posting-for",
            "Last Day to Withdraw Without Transcript Posting for ",
            "Withdraw without transcript posting",
            "final",
            9,
            SummaryKind::Registrar,
        ),
        (
            r"(?i)^40% Refund Deadline",
            "green-river-40-percent-refund-deadline-for",
            "40% Refund Deadline for ",
            "40% refund deadline",
            "final",
            9,
            SummaryKind::Registrar,
        ),
        (
            r"(?i)^Residency Reclassification Deadline",
            "green-river-residency-reclassification-deadline-for",
            "Residency Reclassification Deadline for ",
            "Residency reclassification deadline",
            "final",
            9,
            SummaryKind::Registrar,
        ),
        (
            r"(?i)^Pass/No-Credit Request Deadline",
            "green-river-pass-no-credit-request-deadline-for",
            "Pass/No-Credit Request Deadline for ",
            "Pass/no-credit request deadline",
            "final",
            9,
            SummaryKind::Registrar,
        ),
        (
            r"(?i)^Official Withdrawal Deadline",
            "green-river-official-withdrawal-deadline-for",
            "Official Withdrawal Deadline for ",
            "Official withdrawal deadline",
            "final",
            9,
            SummaryKind::Registrar,
        ),
        (
            r"(?i)^Study Day",
            "green-river-study-day-for",
            "Study Day for ",
            "Study day",
            "final",
            9,
            SummaryKind::Registrar,
        ),
        (
            r"(?i)^Finals",
            "green-river-finals-begin-for",
            "Finals Begin for ",
            "Finals begin",
            "final",
            9,
            SummaryKind::Registrar,
        ),
        (
            r"(?i)^Quarter Ends",
            "green-river-quarter-ends-for",
            "Quarter Ends for ",
            "Quarter ends",
            "final",
            9,
            SummaryKind::Registrar,
        ),
        (
            r"(?i)^Grades Due",
            "green-river-grades-due-for",
            "Grades Due for ",
            "Grades due",
            "final",
            7,
            SummaryKind::Registrar,
        ),
    ];
    rows.into_iter()
        .map(
            |(pattern, id, title_prefix, label, deadline_type, hour, summary_kind)| RegistrarSpec {
                pattern,
                id,
                title_prefix,
                label,
                deadline_type,
                hour,
                summary_kind,
            },
        )
        .collect()
}

fn registrar_summary(kind: SummaryKind, label: &str, terms: &str) -> String {
    match kind {
        SummaryKind::Priority => format!(
            "Priority enrollment opens at 8:00 AM for students with priority registration status for {terms}. Exact appointment access times should be checked in ctcLink/My Green River because dates are subject to change."
        ),
        SummaryKind::Credits90 => format!(
            "Class enrollment opens at 8:00 AM for current or returning students with 90 or more completed/evaluated credits for {terms}. Exact appointment access times should be checked in ctcLink/My Green River because dates are subject to change."
        ),
        SummaryKind::Credits60 => format!(
            "Class enrollment opens at 8:00 AM for current or returning students with 60 to 89 completed/evaluated credits for {terms}. Exact appointment access times should be checked in ctcLink/My Green River because dates are subject to change."
        ),
        SummaryKind::Credits30 => format!(
            "Class enrollment opens at 8:00 AM for current or returning students with 30 to 59 completed/evaluated credits for {terms}. Exact appointment access times should be checked in ctcLink/My Green River because dates are subject to change."
        ),
        SummaryKind::Credits0 => format!(
            "Class enrollment opens at 8:00 AM for current or returning students with 0 to 29 completed/evaluated credits for {terms}. Exact appointment access times should be checked in ctcLink/My Green River because dates are subject to change."
        ),
        SummaryKind::NewStudent => format!(
            "Class enrollment opens for new students assigned an enrollment appointment for {terms} after completing the required admissions/advising steps. Exact appointment access times should be checked in ctcLink/My Green River because dates are subject to change."
        ),
        SummaryKind::Registrar => format!(
            "Green River's registrar calendar lists {} for {}. Dates are subject to change; verify the official registrar page and ctcLink/My Green River when planning registration, payment, or course changes.",
            label.to_lowercase(),
            terms
        ),
    }
}

fn parse_grc_registrar_deadlines(html: &str, now_iso: &str) -> Result<Vec<Value>, String> {
    let rows = extract_table_after(html, "Important Upcoming Dates")?;
    let headers: Vec<TermInfo> = rows
        .first()
        .map(|row| {
            row.iter()
                .skip(1)
                .filter_map(|cell| parse_quarter_header(cell))
                .collect()
        })
        .unwrap_or_default();
    if headers.is_empty()
        || rows
            .first()
            .unwrap()
            .iter()
            .skip(1)
            .any(|cell| parse_quarter_header(cell).is_none())
    {
        return Err("Could not parse Green River registrar quarter headers.".to_string());
    }
    let cutoff = get_cutoff_iso(now_iso)?;
    let specs = registrar_specs();
    let mut raw = Vec::new();
    for row in rows.iter().skip(1) {
        let Some(row_label) = row.first() else {
            continue;
        };
        let Some(spec) = specs
            .iter()
            .find(|spec| Regex::new(spec.pattern).is_ok_and(|regex| regex.is_match(row_label)))
            .cloned()
        else {
            continue;
        };
        for (index, cell) in row.iter().skip(1).enumerate() {
            let Some(term) = headers.get(index) else {
                continue;
            };
            let Some(month_day) = parse_first_month_day(cell) else {
                continue;
            };
            let year = if term.term == "Winter" && month_day.month >= 9 {
                term.year - 1
            } else {
                term.year
            };
            let due_at = zoned_datetime_to_iso(year, month_day.month, month_day.day, spec.hour)?;
            if due_at < cutoff {
                continue;
            }
            raw.push(RawDeadline {
                spec: spec.clone(),
                term: term.clone(),
                due_at,
            });
        }
    }
    let mut groups: Vec<(RegistrarSpec, String, Vec<TermInfo>)> = Vec::new();
    let mut index_by_key = HashMap::<String, usize>::new();
    for item in raw {
        let key = format!("{}|{}", item.spec.id, item.due_at);
        if let Some(index) = index_by_key.get(&key).copied() {
            groups[index].2.push(item.term.clone());
        } else {
            index_by_key.insert(key, groups.len());
            groups.push((
                item.spec.clone(),
                item.due_at.clone(),
                vec![item.term.clone()],
            ));
        }
    }
    groups
        .into_iter()
        .map(|(spec, due_at, mut terms)| {
            sort_terms(&mut terms);
            let terms_display = format_term_group(&terms);
            let term_slug = format_term_group_slug(&terms, false);
            let title = format!("{}{terms_display}", spec.title_prefix);
            let summary = registrar_summary(spec.summary_kind, spec.label, &terms_display);
            Ok(build_base_opportunity(
                format!("{}-{term_slug}", spec.id),
                title,
                "Green River College",
                summary,
                GRC_REGISTRAR.url,
                due_at,
                spec.deadline_type,
                spec.label,
                empty_matching(),
                default_eligibility(),
                grc_college(),
                GRC_REGISTRAR,
                now_iso,
                default_recurrence(),
            ))
        })
        .collect()
}

fn parse_grc_financial_aid_deadlines(html: &str, now_iso: &str) -> Result<Vec<Value>, String> {
    let rows = extract_table_after(html, "Past and Upcoming Priority Funding Processing Dates")?;
    let quarter_regex =
        Regex::new(r"(?i)\b(Winter|Spring|Summer|Fall)\s+Quarter\s+(20\d{2})\b").unwrap();
    let mut raw = Vec::<(TermInfo, String)>::new();
    for row in &rows {
        if row.len() < 2 || row[0].to_ascii_lowercase().contains("start quarter") {
            continue;
        }
        let Some(quarter) = quarter_regex.captures(&row[0]) else {
            continue;
        };
        let Some((date, year)) = parse_explicit_date(&row[1]) else {
            continue;
        };
        raw.push((
            TermInfo {
                term: capitalize_term(quarter.get(1).unwrap().as_str()),
                year: quarter.get(2).unwrap().as_str().parse().unwrap_or_default(),
            },
            zoned_datetime_to_iso(year, date.month, date.day, 9)?,
        ));
    }
    let mut groups: Vec<(String, Vec<TermInfo>)> = Vec::new();
    let mut index_by_due_at = HashMap::<String, usize>::new();
    for (term, due_at) in raw {
        if let Some(index) = index_by_due_at.get(&due_at).copied() {
            groups[index].1.push(term);
        } else {
            index_by_due_at.insert(due_at.clone(), groups.len());
            groups.push((due_at, vec![term]));
        }
    }
    groups.into_iter().map(|(due_at, mut terms)| {
        sort_terms(&mut terms);
        let terms_display = format_term_group(&terms);
        let term_slug = format_term_group_slug(&terms, true);
        Ok(build_base_opportunity(
            format!("green-river-fafsa-wasfa-priority-deadline-for-{term_slug}"),
            format!("Green River FAFSA/WASFA Priority Deadline for {terms_display}"),
            "Green River College Financial Aid Office",
            format!("Complete your FAFSA or WASFA and submit all required Green River financial aid documents by this date to receive priority processing for {terms_display}. Meeting the deadline can help keep your classes from being dropped for non-payment while aid is being processed."),
            GRC_FINANCIAL_AID.url, due_at, "priority", "Priority processing deadline",
            json!({"financialAidTags": ["need_based", "fafsa_required"], "suggestedMajors": [], "hasToBeMajor": false}),
            default_eligibility(), grc_college(), GRC_FINANCIAL_AID, now_iso, default_recurrence(),
        ))
    }).collect()
}

fn get_next_occurrence_iso(month: u32, day: u32, now_iso: &str) -> Result<String, String> {
    let now = DateTime::parse_from_rfc3339(now_iso)
        .map_err(|error| format!("invalid refresh timestamp {now_iso:?}: {error}"))?;
    let local_now = now.with_timezone(&Los_Angeles);
    let mut year = local_now.year();
    let mut due_at = zoned_datetime_to_iso(year, month, day, 9)?;
    if due_at < now.to_utc().to_rfc3339_opts(SecondsFormat::Millis, true) {
        year += 1;
        due_at = zoned_datetime_to_iso(year, month, day, 9)?;
    }
    Ok(due_at)
}

fn parse_uw_seattle_transfer_deadlines(html: &str, now_iso: &str) -> Result<Vec<Value>, String> {
    let rows = extract_table_after(html, "Key dates &amp; deadlines")?;
    let mut raw = Vec::<(String, Option<MonthDay>, MonthDay, String)>::new();
    for row in rows.iter().skip(1) {
        let quarter = row
            .first()
            .map(|value| value.replace('*', "").trim().to_string())
            .unwrap_or_default();
        let opens = row.get(2).and_then(|value| parse_first_month_day(value));
        let Some(deadline) = row.get(3).and_then(|value| parse_first_month_day(value)) else {
            continue;
        };
        if quarter.is_empty() {
            continue;
        }
        let due_at = get_next_occurrence_iso(deadline.month, deadline.day, now_iso)?;
        raw.push((quarter, opens, deadline, due_at));
    }
    let mut groups: Vec<(MonthDay, Option<MonthDay>, String, Vec<String>)> = Vec::new();
    let mut index_by_key = HashMap::<String, usize>::new();
    for (quarter, opens, deadline, due_at) in raw {
        let key = format!(
            "{}-{}|{}-{}",
            deadline.month,
            deadline.day,
            opens.map(|value| value.month).unwrap_or_default(),
            opens.map(|value| value.day).unwrap_or_default()
        );
        if let Some(index) = index_by_key.get(&key).copied() {
            groups[index].3.push(quarter);
        } else {
            index_by_key.insert(key, groups.len());
            groups.push((deadline, opens, due_at, vec![quarter]));
        }
    }
    groups.into_iter().map(|(deadline, opens, due_at, quarters)| {
        let quarter_display = quarters.join("/");
        let quarter_slug = slugify(&quarter_display);
        let opens_text = opens.map(|date| format!("{} {}", month_name(date.month), date.day)).unwrap_or_else(|| "the published opening date".to_string());
        let deadline_text = format!("{} {}", month_name(deadline.month), deadline.day);
        let spring_note = if quarters.iter().any(|quarter| quarter == "Spring") { " Spring quarter is limited to applicants to specific departments in engineering and computer science." } else { "" };
        let summer_note = if quarters.iter().any(|quarter| quarter == "Summer") { " Most degree-seeking transfer students apply for Autumn quarter; Summer admission has special notes on the UW page." } else { "" };
        Ok(build_base_opportunity(
            format!("uw-seattle-transfer-application-deadline-for-{quarter_slug}-admission"),
            format!("UW Seattle Transfer Application Deadline for {quarter_display} Admission"),
            "UW Seattle Office of Admissions",
            format!("UW Seattle transfer applications for {quarter_display} admission open {opens_text} and are due {deadline_text} each year.{spring_note}{summer_note}"),
            UW_SEATTLE_TRANSFER.url, due_at, "final", "Transfer application deadline", empty_matching(),
            json!({"gpaMin": null, "residencyTypes": [], "transferOnly": true}), uw_college(), UW_SEATTLE_TRANSFER, now_iso,
            json!({"isYearly": true, "month": deadline.month, "day": deadline.day, "timezone": TIMEZONE}),
        ))
    }).collect()
}

fn month_name(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "",
    }
}

#[derive(Clone)]
enum UpdateResult {
    Updated(Value),
    Unchanged(Value),
}

fn apply_opportunity_update(existing: &Value, candidate: &Value, now_iso: &str) -> UpdateResult {
    if canonicalize_json(semantic_opportunity(existing))
        == canonicalize_json(semantic_opportunity(candidate))
    {
        return UpdateResult::Unchanged(existing.clone());
    }
    let mut updated = candidate.clone();
    let created_at = existing
        .get("createdAt")
        .cloned()
        .filter(|value| !value.is_null())
        .unwrap_or(json!(now_iso));
    if let Some(object) = updated.as_object_mut() {
        object.insert("createdAt".to_string(), created_at);
        object.insert("updatedAt".to_string(), json!(now_iso));
        if let Some(source) = object.get_mut("source").and_then(Value::as_object_mut) {
            source.insert("fetchedAt".to_string(), json!(now_iso));
            source.insert("verifiedAt".to_string(), json!(now_iso));
        }
    }
    UpdateResult::Updated(updated)
}

fn semantic_opportunity(value: &Value) -> Value {
    let mut clone = value.clone();
    if let Some(object) = clone.as_object_mut() {
        object.insert("createdAt".to_string(), Value::Null);
        object.insert("updatedAt".to_string(), Value::Null);
        if let Some(source) = object.get_mut("source").and_then(Value::as_object_mut) {
            source.insert("fetchedAt".to_string(), Value::Null);
            source.insert("verifiedAt".to_string(), Value::Null);
        }
    }
    clone
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_json).collect()),
        Value::Object(object) => {
            let mut entries: Vec<_> = object.into_iter().collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let mut canonical = serde_json::Map::new();
            for (key, value) in entries {
                canonical.insert(key, canonicalize_json(value));
            }
            Value::Object(canonical)
        }
        scalar => scalar,
    }
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, contents)
        .map_err(|error| format!("could not write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path)
        .map_err(|error| format!("could not replace {}: {error}", path.display()))
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn write_report(
    report: &DeadlineRefreshReport,
    json_path: &Path,
    markdown_path: &Path,
) -> Result<(), String> {
    let mut encoded = serde_json::to_string_pretty(report).map_err(|error| error.to_string())?;
    encoded.push('\n');
    fs::write(json_path, encoded).map_err(|error| {
        format!(
            "could not write deadline report {}: {error}",
            json_path.display()
        )
    })?;
    let mut lines = vec![
        "# Deadline Refresh Report".to_string(),
        String::new(),
        format!("- Checked at: {}", report.checked_at),
        format!("- Parsed deadlines: {}", report.parsed_count),
        format!("- Created: {}", report.created.len()),
        format!("- Updated: {}", report.updated.len()),
        format!("- Unchanged: {}", report.unchanged.len()),
        String::new(),
        "## Sources".to_string(),
        String::new(),
    ];
    lines.extend(report.sources.iter().map(|source| {
        format!(
            "- {}: {} parsed from {}",
            source.label, source.parsed_count, source.url
        )
    }));
    lines.extend([String::new(), "## Created".to_string(), String::new()]);
    if report.created.is_empty() {
        lines.push("- None".to_string());
    } else {
        lines.extend(report.created.iter().map(|id| format!("- {id}")));
    }
    lines.extend([String::new(), "## Updated".to_string(), String::new()]);
    if report.updated.is_empty() {
        lines.push("- None".to_string());
    } else {
        lines.extend(report.updated.iter().map(|id| format!("- {id}")));
    }
    lines.push(String::new());
    fs::write(markdown_path, lines.join("\n") + "\n").map_err(|error| {
        format!(
            "could not write deadline summary {}: {error}",
            markdown_path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-01-15T20:00:00.000Z";

    #[test]
    fn registrar_parser_groups_same_date_terms_and_preserves_enrollment_priority() {
        let html = r#"<h2>Important Upcoming Dates</h2><table>
          <tr><th>Date</th><th>Summer 2026</th><th>Fall 2026</th></tr>
          <tr><td>Priority Enrollment</td><td>Mar 1</td><td>Mar 1</td></tr>
          <tr><td>Quarter Begins</td><td>Jun 22</td><td>Sep 21</td></tr>
        </table>"#;
        let items = parse_grc_registrar_deadlines(html, NOW).unwrap();
        assert_eq!(items.len(), 3);
        let grouped = items
            .iter()
            .find(|item| {
                item["opportunityId"]
                    .as_str()
                    .unwrap()
                    .starts_with("green-river-priority-enrollment")
            })
            .unwrap();
        assert_eq!(
            grouped["title"],
            "Priority Class Enrollment for Summer/Fall 2026"
        );
        assert_eq!(grouped["deadline"]["type"], "priority");
        assert_eq!(
            grouped["summary"],
            "Priority enrollment opens at 8:00 AM for students with priority registration status for Summer/Fall 2026. Exact appointment access times should be checked in ctcLink/My Green River because dates are subject to change."
        );
        assert_eq!(grouped["dueAt"], "2026-03-01T16:00:00.000Z");
        assert_eq!(
            grouped
                .as_object()
                .unwrap()
                .keys()
                .next()
                .map(String::as_str),
            Some("schemaVersion")
        );
    }

    #[test]
    fn registrar_winter_dates_in_the_prior_calendar_year_are_scoped_correctly() {
        let html = r#"<h2>Important Upcoming Dates</h2><table><tr><th>Date</th><th>Winter 2027</th></tr><tr><td>Quarter Begins</td><td>Sep 28</td></tr></table>"#;
        let items = parse_grc_registrar_deadlines(html, "2026-09-01T12:00:00.000Z").unwrap();
        assert_eq!(items[0]["dueAt"], "2026-09-28T16:00:00.000Z");
    }

    #[test]
    fn financial_aid_dates_merge_quarters_with_same_deadline() {
        let html = r#"<h2>Past and Upcoming Priority Funding Processing Dates</h2><table>
          <tr><th>Start Quarter</th><th>Date</th></tr>
          <tr><td>Winter Quarter 2026</td><td>February 1, 2026</td></tr>
          <tr><td>Spring Quarter 2026</td><td>February 1, 2026</td></tr>
        </table>"#;
        let items = parse_grc_financial_aid_deadlines(html, NOW).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0]["opportunityId"],
            "green-river-fafsa-wasfa-priority-deadline-for-winter-and-spring-2026"
        );
        assert_eq!(items[0]["dueAt"], "2026-02-01T17:00:00.000Z");
    }

    #[test]
    fn uw_deadline_rows_group_quarters_and_create_yearly_local_recurrence() {
        let html = r#"<h2>Key dates &amp; deadlines</h2><table>
          <tr><th>Quarter</th><th>Applications open</th><th>Priority date</th><th>Deadline</th></tr>
          <tr><td>Autumn</td><td>Applications open</td><td>September 1</td><td>December 15</td></tr>
          <tr><td>Winter</td><td>Applications open</td><td>September 1</td><td>December 15</td></tr>
          <tr><td>Spring*</td><td>Applications open</td><td>December 15</td><td>February 15</td></tr>
        </table>"#;
        let items = parse_uw_seattle_transfer_deadlines(html, NOW).unwrap();
        assert_eq!(items.len(), 2);
        let grouped = items
            .iter()
            .find(|item| item["title"].as_str().unwrap().contains("Autumn/Winter"))
            .unwrap();
        assert_eq!(
            grouped["recurrence"],
            json!({"isYearly": true, "month": 12, "day": 15, "timezone": TIMEZONE})
        );
        assert_eq!(grouped["dueAt"], "2026-12-15T17:00:00.000Z");
    }

    #[test]
    fn semantic_comparison_ignores_fetch_timestamps_but_updates_preserve_created_at() {
        let existing = json!({"opportunityId":"x","createdAt":"old-created","updatedAt":"old-updated","source":{"fetchedAt":"old-fetch","verifiedAt":"old-verified"},"title":"old"});
        let same = json!({"title":"old","source":{"verifiedAt":"new-verified","fetchedAt":"new-fetch"},"updatedAt":"new-updated","createdAt":"new-created","opportunityId":"x"});
        assert!(matches!(
            apply_opportunity_update(&existing, &same, NOW),
            UpdateResult::Unchanged(_)
        ));
        let changed = json!({"opportunityId":"x","createdAt":"new-created","updatedAt":"new-updated","source":{"fetchedAt":"new-fetch","verifiedAt":"new-verified"},"title":"new"});
        let UpdateResult::Updated(updated) = apply_opportunity_update(&existing, &changed, NOW)
        else {
            panic!("expected update")
        };
        assert_eq!(updated["createdAt"], "old-created");
        assert_eq!(updated["updatedAt"], NOW);
    }
}
