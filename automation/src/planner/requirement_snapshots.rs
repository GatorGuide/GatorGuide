use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementSourceSnapshot {
    pub snapshot_path: PathBuf,
    pub source_url: Option<String>,
    pub title: Option<String>,
    pub has_heading_metadata: bool,
    pub headings: Vec<String>,
    pub snapshot_lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRecoveryArtifacts {
    pub html: String,
    pub title: Option<String>,
    pub headings: Vec<String>,
    pub lines: Vec<String>,
    pub snapshot_path: Option<PathBuf>,
    pub from_snapshot: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RequirementSnapshotStore {
    directory: PathBuf,
}

impl RequirementSnapshotStore {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn write_snapshot(
        &self,
        owner_key: &str,
        source_url: &str,
        title: Option<&str>,
        lines: &[String],
        headings: &[String],
    ) -> Result<PathBuf, String> {
        fs::create_dir_all(&self.directory).map_err(|error| {
            format!(
                "could not create requirement snapshot directory {}: {error}",
                self.directory.display()
            )
        })?;
        let output_path = self.snapshot_path(owner_key, Some(source_url));
        let heading_values = unique_in_order(
            headings
                .iter()
                .map(|heading| normalize_whitespace(heading))
                .filter(|heading| !heading.is_empty()),
        )
        .into_iter()
        .take(40)
        .collect::<Vec<_>>();
        let mut body = vec![
            format!("Owner: {owner_key}"),
            format!("Source: {source_url}"),
            format!("Title: {}", title.unwrap_or("")),
            format!(
                "Headings: {}",
                serde_json::to_string(&heading_values).map_err(|error| error.to_string())?
            ),
            String::new(),
        ];
        body.extend(lines.iter().cloned());
        fs::write(&output_path, format!("{}\n", body.join("\n"))).map_err(|error| {
            format!(
                "could not write requirement snapshot {}: {error}",
                output_path.display()
            )
        })?;
        Ok(output_path)
    }

    pub fn snapshot_path(&self, owner_key: &str, source_url: Option<&str>) -> PathBuf {
        let source_hash = source_url.and_then(snapshot_source_hash);
        let suffix = source_hash
            .map(|hash| format!("-{hash}"))
            .unwrap_or_default();
        self.directory
            .join(format!("{}{suffix}.txt", slugify(owner_key)))
    }

    pub fn read_snapshot_file(
        &self,
        path: &Path,
        expected_source_url: Option<&str>,
    ) -> Option<RequirementSourceSnapshot> {
        let raw = fs::read_to_string(path).ok()?;
        let raw_lines = raw
            .split('\n')
            .map(|line| line.trim_end_matches('\r'))
            .collect::<Vec<_>>();
        let source_line = raw_lines.iter().find(|line| line.starts_with("Source:"));
        let source_url =
            source_line.map(|line| normalize_whitespace(line.trim_start_matches("Source:").trim()));
        if let (Some(expected), Some(actual)) = (expected_source_url, source_url.as_deref()) {
            if !actual.is_empty() {
                let expected_exact = normalize_snapshot_source_url(expected);
                let actual_exact = normalize_snapshot_source_url(actual);
                let expected_base = source_url_without_hash(expected);
                let actual_base = source_url_without_hash(actual);
                if expected_exact != actual_exact
                    && (source_url_has_hash(expected) || expected_base != actual_base)
                {
                    return None;
                }
            }
        }

        let title_line = raw_lines.iter().find(|line| line.starts_with("Title:"));
        let headings_line = raw_lines.iter().find(|line| line.starts_with("Headings:"));
        let body_start = raw_lines
            .iter()
            .enumerate()
            .find(|(index, line)| *index <= 6 && line.trim().is_empty())
            .map(|(index, _)| index + 1)
            .unwrap_or(0);
        let snapshot_lines = raw_lines
            .iter()
            .skip(body_start)
            .map(|line| normalize_whitespace(line))
            .filter(|line| !line.is_empty() && !noisy_line_pattern().is_match(line))
            .take(1200)
            .collect::<Vec<_>>();
        if snapshot_lines.is_empty() {
            return None;
        }
        Some(RequirementSourceSnapshot {
            snapshot_path: path.to_path_buf(),
            source_url,
            title: title_line
                .map(|line| normalize_whitespace(line.trim_start_matches("Title:").trim())),
            has_heading_metadata: headings_line.is_some(),
            headings: headings_line
                .map(|line| parse_heading_metadata(line.trim_start_matches("Headings:").trim()))
                .unwrap_or_default(),
            snapshot_lines,
        })
    }

    pub fn find_snapshot_by_source_url(
        &self,
        source_url: &str,
    ) -> Option<RequirementSourceSnapshot> {
        source_url_without_hash(source_url)?;
        for entry in fs::read_dir(&self.directory).ok()? {
            let path = entry.ok()?.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("txt") {
                continue;
            }
            if let Some(snapshot) = self.read_snapshot_file(&path, Some(source_url)) {
                return Some(snapshot);
            }
        }
        None
    }

    pub fn read_snapshot(
        &self,
        owner_key: &str,
        source_url: Option<&str>,
    ) -> Option<RequirementSourceSnapshot> {
        self.read_snapshot_file(&self.snapshot_path(owner_key, source_url), source_url)
            .or_else(|| self.read_snapshot_file(&self.snapshot_path(owner_key, None), source_url))
            .or_else(|| source_url.and_then(|url| self.find_snapshot_by_source_url(url)))
    }

    pub fn read_owner_snapshots(&self, owner_key: &str) -> Vec<RequirementSourceSnapshot> {
        let slug = slugify(owner_key);
        if slug.is_empty() {
            return Vec::new();
        }
        let Ok(entries) = fs::read_dir(&self.directory) else {
            return Vec::new();
        };
        let exact_name = format!("{slug}.txt");
        let hashed_prefix = format!("{slug}-");
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().and_then(|extension| extension.to_str()) == Some("txt")
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name == exact_name || name.starts_with(&hashed_prefix))
            })
            .filter_map(|path| self.read_snapshot_file(&path, None))
            .collect()
    }

    pub fn build_parser_recovery_artifacts(
        &self,
        owner_key: &str,
        entry_source_url: Option<&str>,
        owner_source_url: Option<&str>,
        owner_snapshot_lines: Option<&[String]>,
        owner_snapshot_path: Option<&Path>,
        extracted_title: Option<&str>,
        extracted_headings: Option<&[String]>,
        source_label: Option<&str>,
        entry_label: Option<&str>,
    ) -> Option<SnapshotRecoveryArtifacts> {
        let source_url = owner_source_url.or(entry_source_url);
        let snapshot = owner_snapshot_lines
            .filter(|lines| !lines.is_empty())
            .map(|lines| RequirementSourceSnapshot {
                snapshot_path: owner_snapshot_path
                    .map(|path| path.to_path_buf())
                    .unwrap_or_default(),
                source_url: source_url.map(str::to_string),
                title: extracted_title.map(str::to_string),
                has_heading_metadata: extracted_headings
                    .is_some_and(|headings| !headings.is_empty()),
                headings: extracted_headings.unwrap_or_default().to_vec(),
                snapshot_lines: lines.to_vec(),
            })
            .or_else(|| {
                owner_snapshot_path.and_then(|path| {
                    self.read_snapshot_file(path, source_url)
                        .or_else(|| self.read_snapshot_file(path, None))
                })
            })
            .or_else(|| self.read_snapshot(owner_key, source_url))?;
        let synthetic_links = build_synthetic_snapshot_link_html(&snapshot.snapshot_lines);
        let mut html_parts = vec![snapshot.snapshot_lines.join("\n")];
        if !synthetic_links.is_empty() {
            html_parts.push(synthetic_links);
        }
        Some(SnapshotRecoveryArtifacts {
            html: html_parts.join("\n"),
            title: extracted_title
                .map(str::to_string)
                .or_else(|| snapshot.title.clone())
                .or_else(|| source_label.map(str::to_string))
                .or_else(|| entry_label.map(str::to_string)),
            headings: extracted_headings
                .filter(|headings| !headings.is_empty())
                .map(|headings| headings.to_vec())
                .unwrap_or_else(|| snapshot.headings.clone()),
            lines: snapshot.snapshot_lines,
            snapshot_path: if snapshot.snapshot_path.as_os_str().is_empty() {
                owner_snapshot_path.map(|path| path.to_path_buf())
            } else {
                Some(snapshot.snapshot_path)
            },
            from_snapshot: true,
        })
    }

    pub fn build_recovery_artifacts(
        &self,
        owner_key: &str,
        source_url: Option<&str>,
        title_fallback: Option<&str>,
        headings_fallback: Option<&[String]>,
    ) -> Option<SnapshotRecoveryArtifacts> {
        self.build_parser_recovery_artifacts(
            owner_key,
            source_url,
            None,
            None,
            None,
            title_fallback,
            headings_fallback,
            None,
            None,
        )
    }
}

pub fn normalize_snapshot_source_url(value: &str) -> Option<String> {
    let parsed = Url::parse(value).ok()?;
    let path = parsed.path().trim_end_matches('/');
    let path = if path.is_empty() { "/" } else { path };
    let mut normalized = format!("{}://{}", parsed.scheme(), parsed.host_str()?);
    if let Some(port) = parsed.port() {
        let default = matches!((parsed.scheme(), port), ("http", 80) | ("https", 443));
        if !default {
            normalized.push(':');
            normalized.push_str(&port.to_string());
        }
    }
    normalized.push_str(path);
    if let Some(query) = parsed.query().filter(|query| !query.is_empty()) {
        normalized.push('?');
        normalized.push_str(query);
    }
    if let Some(fragment) = parsed.fragment().filter(|fragment| !fragment.is_empty()) {
        normalized.push('#');
        normalized.push_str(fragment);
    }
    Some(normalized)
}

pub fn source_url_without_hash(value: &str) -> Option<String> {
    let normalized = normalize_snapshot_source_url(value)?;
    Some(
        normalized
            .split('#')
            .next()
            .unwrap_or(&normalized)
            .to_string(),
    )
}

pub fn source_url_has_hash(value: &str) -> bool {
    Url::parse(value)
        .ok()
        .and_then(|parsed| parsed.fragment().map(|fragment| !fragment.is_empty()))
        .unwrap_or(false)
}

pub fn parse_heading_metadata(value: &str) -> Vec<String> {
    let value = value.trim();
    if value.is_empty() {
        return Vec::new();
    }
    if let Ok(parsed) = serde_json::from_str::<Value>(value) {
        if let Some(array) = parsed.as_array() {
            return unique_in_order(
                array
                    .iter()
                    .map(json_heading_text)
                    .map(|heading| normalize_whitespace(&heading))
                    .filter(|heading| !heading.is_empty()),
            );
        }
    }
    unique_in_order(
        heading_separator_pattern()
            .split(value)
            .map(normalize_whitespace)
            .filter(|heading| !heading.is_empty()),
    )
}

fn snapshot_source_hash(source_url: &str) -> Option<String> {
    let normalized = normalize_snapshot_source_url(source_url)?;
    let digest = sha1(normalized.as_bytes());
    Some(
        digest[..5]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

fn build_synthetic_snapshot_link_html(snapshot_lines: &[String]) -> String {
    let mut anchors = Vec::new();
    for (index, line) in snapshot_lines.iter().enumerate() {
        let href = snapshot_href_value(line);
        if href.is_empty() || unsafe_snapshot_href_pattern().is_match(&href) {
            continue;
        }
        let label = find_snapshot_link_label(snapshot_lines, index, &href);
        anchors.push(format!(
            "<a href=\"{}\">{}</a>",
            escape_html_attribute(&href),
            escape_html_text(&label)
        ));
    }
    anchors.join("\n")
}

fn find_snapshot_link_label(lines: &[String], href_index: usize, href: &str) -> String {
    let same_line = snapshot_href_attribute_pattern()
        .replace(&lines[href_index], " ")
        .into_owned();
    let same_line = strip_html(&same_line);
    if !snapshot_link_label_noise(&same_line) && same_line.len() >= 4 {
        return same_line;
    }
    let mut parts = Vec::new();
    for candidate in lines.iter().skip(href_index + 1).take(6) {
        let candidate = strip_html(candidate);
        if snapshot_link_label_noise(&candidate) {
            continue;
        }
        let long_enough = candidate.len() >= 8;
        parts.push(candidate);
        if long_enough {
            break;
        }
    }
    let label = normalize_whitespace(&parts.join(" "));
    if label.is_empty() {
        href.to_string()
    } else {
        label
    }
}

fn snapshot_href_value(line: &str) -> String {
    snapshot_href_pattern()
        .captures(line)
        .and_then(|captures| {
            captures
                .get(1)
                .or_else(|| captures.get(2))
                .or_else(|| captures.get(3))
        })
        .map(|value| normalize_whitespace(value.as_str()))
        .unwrap_or_default()
}

fn snapshot_link_label_noise(line: &str) -> bool {
    line.is_empty()
        || snapshot_tag_only_pattern().is_match(line)
        || snapshot_attribute_line_pattern().is_match(line)
        || snapshot_src_href_line_pattern().is_match(line)
        || snapshot_angle_only_pattern().is_match(line)
}

fn escape_html_attribute(value: &str) -> String {
    escape_html_text(value).replace('"', "&quot;")
}

fn escape_html_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn strip_html(value: &str) -> String {
    let without_tags = html_tag_pattern().replace_all(value, " ");
    let decoded = super::requirement_html::decode_transfer_planner_html_entities(&without_tags);
    normalize_whitespace(&decoded)
}

fn slugify(value: &str) -> String {
    let normalized = normalize_whitespace(value).to_lowercase();
    let mut output = String::with_capacity(normalized.len());
    let mut previous_hyphen = false;
    for character in normalized.chars() {
        if character.is_ascii_alphanumeric() {
            output.push(character);
            previous_hyphen = false;
        } else if !previous_hyphen && !output.is_empty() {
            output.push('-');
            previous_hyphen = true;
        }
    }
    while output.ends_with('-') {
        output.pop();
    }
    output
}

fn json_heading_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => super::requirement_shape::js_number_to_string(value),
        Value::Array(values) => values
            .iter()
            .map(json_heading_text)
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

fn normalize_whitespace(value: &str) -> String {
    super::requirement_shape::normalize_js_whitespace(value)
}

fn unique_in_order(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut output = Vec::new();
    for value in values {
        if !value.is_empty() && !output.contains(&value) {
            output.push(value);
        }
    }
    output
}

fn heading_separator_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_snapshot_pattern(r"\s*\|\s*"))
}
fn noisy_line_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_snapshot_pattern(r"(?i)(?:src=|srcset=|aria-label=|title=|\.jpg\b|\.jpeg\b|\.png\b|\.svg\b|\.gif\b|^\[page\s+\d+\]\s*%pdf-|^\s*%pdf-|^\s*(?:<!--|-->|(?:pre-bubbling\s+)?cache\s+(?:contexts|tags|max-age):?$|(?:\*\s*)?(?:route\.name\.|url\.site$|languages:|user\.permissions$|theme$)))"))
}
fn snapshot_href_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_snapshot_pattern(r#"(?i)\bhref\s*=\s*(?:"([^"]+)"|'([^']+)'|([^\s>]+))"#)
    })
}
fn snapshot_href_attribute_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_snapshot_pattern(r#"(?i)\bhref\s*=\s*(?:"[^"]+"|'[^']+'|[^\s>]+)"#)
    })
}
fn snapshot_tag_only_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_snapshot_pattern(r"(?i)^</?[a-z][^>]*>?$"))
}
fn snapshot_attribute_line_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        js_compatible_snapshot_pattern(
            r"(?i)^(?:class|style|target|rel|aria-[\w-]+|data-[\w-]+|id|title)\s*=",
        )
    })
}
fn snapshot_src_href_line_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_snapshot_pattern(r"(?i)^(?:href|src)\s*="))
}
fn snapshot_angle_only_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_snapshot_pattern(r"^[<>/]+$"))
}
fn unsafe_snapshot_href_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_snapshot_pattern(r"(?i)^(?:#|javascript:|mailto:|tel:)"))
}
fn html_tag_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| js_compatible_snapshot_pattern(r"<[^>]+>"))
}

fn js_compatible_snapshot_pattern(pattern: &str) -> Regex {
    let compatible_pattern = super::requirement_shape::js_regex_compatibility_pattern(pattern);
    Regex::new(&compatible_pattern).expect("JavaScript-compatible snapshot pattern")
}

fn sha1(input: &[u8]) -> [u8; 20] {
    let bit_length = (input.len() as u64).wrapping_mul(8);
    let mut bytes = input.to_vec();
    bytes.push(0x80);
    while bytes.len() % 64 != 56 {
        bytes.push(0);
    }
    bytes.extend_from_slice(&bit_length.to_be_bytes());
    let mut state = [
        0x67452301u32,
        0xefcdab89,
        0x98badcfe,
        0x10325476,
        0xc3d2e1f0,
    ];
    for block in bytes.chunks_exact(64) {
        let mut words = [0u32; 80];
        for (index, chunk) in block.chunks_exact(4).enumerate() {
            words[index] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for index in 16..80 {
            words[index] =
                (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16])
                    .rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = state;
        for (index, word) in words.iter().enumerate() {
            let (function, constant) = match index {
                0..=19 => ((b & c) | ((!b) & d), 0x5a827999),
                20..=39 => (b ^ c ^ d, 0x6ed9eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            let temporary = a
                .rotate_left(5)
                .wrapping_add(function)
                .wrapping_add(e)
                .wrapping_add(constant)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temporary;
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
    }
    let mut digest = [0u8; 20];
    for (index, value) in state.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&value.to_be_bytes());
    }
    digest
}
