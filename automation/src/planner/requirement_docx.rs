use super::requirement_html::decode_transfer_planner_html_entities;
use super::source_artifact_cache::{BoundedResultCache, DEFAULT_SOURCE_ARTIFACT_CACHE_MAX_ENTRIES};
use super::requirement_source_fetching::{
    SOURCE_DOWNLOAD_MAX_BUFFER_BYTES, SourceDownloader, run_command_bounded_with_timeout,
};
use regex::Regex;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

const DOCX_BLOCK_END_TAGS: &[&str] = &["/w:p", "/w:tr", "/w:tbl", "/w:sectpr", "/w:tc"];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocxSourceArtifacts {
    pub title: Option<String>,
    pub lines: Vec<String>,
}

#[derive(Clone)]
pub struct DocxSourceParser {
    downloader: SourceDownloader,
    temporary_root: PathBuf,
    cache: Arc<BoundedResultCache<DocxSourceArtifacts>>,
}

struct TemporaryDirectory(PathBuf);

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl DocxSourceParser {
    pub fn new(downloader: SourceDownloader, temporary_root: impl Into<PathBuf>) -> Self {
        Self::with_cache_capacity(
            downloader,
            temporary_root,
            DEFAULT_SOURCE_ARTIFACT_CACHE_MAX_ENTRIES,
        )
    }

    pub fn with_cache_capacity(
        downloader: SourceDownloader,
        temporary_root: impl Into<PathBuf>,
        max_cache_entries: usize,
    ) -> Self {
        Self {
            downloader,
            temporary_root: temporary_root.into(),
            cache: Arc::new(BoundedResultCache::new(max_cache_entries)),
        }
    }

    pub fn get_docx_source_artifacts(
        &self,
        url: &str,
        timeout: Duration,
    ) -> Result<DocxSourceArtifacts, String> {
        self.cache
            .get_or_load(url, || self.load_docx_source_artifacts(url, timeout))
    }

    fn load_docx_source_artifacts(
        &self,
        url: &str,
        timeout: Duration,
    ) -> Result<DocxSourceArtifacts, String> {
        let downloaded = self.downloader.download_source(url, timeout, true)?;
        let document_xml = extract_docx_document_xml(
            &downloaded.body,
            timeout,
            &self.temporary_root,
            SOURCE_DOWNLOAD_MAX_BUFFER_BYTES,
        )?;
        let lines = build_docx_lines(&document_xml);
        Ok(DocxSourceArtifacts {
            title: lines.first().cloned(),
            lines,
        })
    }
}

pub fn is_docx_source_url(url: &str) -> bool {
    docx_url_pattern().is_match(url)
}

pub fn is_pdf_source_url(url: &str) -> bool {
    pdf_url_pattern().is_match(url)
}

pub fn is_linked_document_source_url(url: &str) -> bool {
    is_pdf_source_url(url) || is_docx_source_url(url)
}

pub fn get_linked_document_parser_type(url: &str, label: &str) -> &'static str {
    if worksheet_label_pattern().is_match(label) {
        "pdf-worksheet"
    } else {
        let _ = url;
        "pdf-degree-sheet"
    }
}

pub fn extract_docx_text(xml: &str) -> String {
    let bytes = xml.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'<' {
            let Some(relative_end) = bytes[index + 1..].iter().position(|byte| *byte == b'>') else {
                break;
            };
            let tag_end = index + 1 + relative_end;
            let tag = String::from_utf8_lossy(&bytes[index + 1..tag_end]);
            let mut tag_name = tag
                .trim()
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if tag_name.ends_with('/') {
                tag_name.pop();
            }
            match tag_name.as_str() {
                "w:tab" | "w:sym" => output.push(b' '),
                "w:br" | "w:cr" => output.push(b'\n'),
                "w:nobreakhyphen" => output.push(b'-'),
                name if DOCX_BLOCK_END_TAGS.contains(&name) => output.push(b'\n'),
                _ => {}
            }
            index = tag_end + 1;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

pub fn build_docx_lines(document_xml: &str) -> Vec<String> {
    extract_docx_text(document_xml)
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .map(decode_transfer_planner_html_entities)
        .map(|line| line.chars().filter(|character| *character != '<' && *character != '>').collect::<String>())
        .map(|line| normalize_whitespace(&line))
        .filter(|line| !line.is_empty())
        .collect()
}

pub fn extract_docx_document_xml(
    docx_buffer: &[u8],
    timeout: Duration,
    temporary_root: &Path,
    max_buffer_bytes: usize,
) -> Result<String, String> {
    fs::create_dir_all(temporary_root).map_err(|error| {
        format!("could not create temporary DOCX extraction root {}: {error}", temporary_root.display())
    })?;
    let temp_dir = TemporaryDirectory(create_unique_temp_dir(temporary_root)?);
    let archive_path = temp_dir.0.join("source.docx");
    fs::write(&archive_path, docx_buffer).map_err(|error| {
        format!("could not write temporary DOCX archive {}: {error}", archive_path.display())
    })?;
    let output = run_command_bounded_with_timeout(
        Command::new("tar").args([
            "-xOf".to_string(),
            archive_path.to_string_lossy().into_owned(),
            "word/document.xml".to_string(),
        ]),
        max_buffer_bytes,
        Some(timeout),
    )
    .map_err(|error| format!("could not extract DOCX document.xml: {error}"))?;
    if output.stdout.len() > max_buffer_bytes || output.stderr.len() > max_buffer_bytes {
        return Err(format!("DOCX extraction exceeded the {max_buffer_bytes}-byte buffer limit"));
    }
    if !output.status.success() {
        let details = normalize_whitespace(&String::from_utf8_lossy(&output.stderr))
            .or_else(|| normalize_whitespace(&String::from_utf8_lossy(&output.stdout)))
            .unwrap_or_else(|| format!("tar exited with status {}", output.status));
        return Err(details);
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn create_unique_temp_dir(parent: &Path) -> Result<PathBuf, String> {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    for _ in 0..100 {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            "transfer-planner-docx-{}-{id}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!("could not create temporary DOCX directory {}: {error}", path.display()));
            }
        }
    }
    Err(format!("could not allocate a unique temporary DOCX directory under {}", parent.display()))
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn docx_url_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\.docx(?:$|[?#])").expect("DOCX URL pattern"))
}
fn pdf_url_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)\.pdf(?:$|[?#])").expect("PDF URL pattern"))
}
fn worksheet_label_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:worksheet|check\s*list|checklist)\b")
            .expect("worksheet parser type label pattern")
    })
}
