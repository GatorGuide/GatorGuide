use super::requirement_course_codes::CourseCodeParser;
use super::requirement_pdf_layout::{
    PdfPageBlock, PdfTextContent, PdfTextItem, build_pdf_page_line_texts,
};
use super::requirement_source_fetching::{
    SOURCE_DOWNLOAD_MAX_BUFFER_BYTES, SourceDownloader, run_command_bounded_with_timeout,
};
use super::source_artifact_cache::{BoundedResultCache, DEFAULT_SOURCE_ARTIFACT_CACHE_MAX_ENTRIES};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const PDFJS_OUTPUT_MAX_BUFFER_BYTES: usize = SOURCE_DOWNLOAD_MAX_BUFFER_BYTES;

#[derive(Clone)]
pub struct PdfSourceParser {
    downloader: SourceDownloader,
    node_command: String,
    helper_script: PathBuf,
    temporary_root: PathBuf,
    cache: Arc<BoundedResultCache<Vec<PdfPageBlock>>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExtractedPdfPage {
    page_number: usize,
    #[serde(default)]
    items: Vec<PdfTextItem>,
}

struct TemporaryDirectory(PathBuf);

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl PdfSourceParser {
    pub fn new(
        downloader: SourceDownloader,
        node_command: impl Into<String>,
        helper_script: impl Into<PathBuf>,
        temporary_root: impl Into<PathBuf>,
    ) -> Self {
        Self::with_cache_capacity(
            downloader,
            node_command,
            helper_script,
            temporary_root,
            DEFAULT_SOURCE_ARTIFACT_CACHE_MAX_ENTRIES,
        )
    }

    pub fn with_cache_capacity(
        downloader: SourceDownloader,
        node_command: impl Into<String>,
        helper_script: impl Into<PathBuf>,
        temporary_root: impl Into<PathBuf>,
        max_cache_entries: usize,
    ) -> Self {
        Self {
            downloader,
            node_command: node_command.into(),
            helper_script: helper_script.into(),
            temporary_root: temporary_root.into(),
            cache: Arc::new(BoundedResultCache::new(max_cache_entries)),
        }
    }

    pub fn get_pdf_page_blocks(
        &self,
        url: &str,
        timeout: Duration,
        course_parser: &CourseCodeParser,
    ) -> Result<Vec<PdfPageBlock>, String> {
        self.cache.get_or_load(url, || {
            self.load_pdf_page_blocks(url, timeout, course_parser)
        })
    }

    fn load_pdf_page_blocks(
        &self,
        url: &str,
        timeout: Duration,
        course_parser: &CourseCodeParser,
    ) -> Result<Vec<PdfPageBlock>, String> {
        let downloaded = self.downloader.download_source(url, timeout, true)?;
        if !self.helper_script.is_file() {
            return Err(format!(
                "PDF.js text-content helper was not found: {}",
                self.helper_script.display()
            ));
        }
        fs::create_dir_all(&self.temporary_root).map_err(|error| {
            format!(
                "could not create temporary PDF extraction root {}: {error}",
                self.temporary_root.display()
            )
        })?;
        let temporary_directory = TemporaryDirectory(create_unique_temp_dir(&self.temporary_root)?);
        let pdf_path = temporary_directory.0.join("source.pdf");
        fs::write(&pdf_path, downloaded.body).map_err(|error| {
            format!(
                "could not write temporary PDF source {}: {error}",
                pdf_path.display()
            )
        })?;

        let output = run_command_bounded_with_timeout(
            Command::new(&self.node_command).args([
                self.helper_script.as_os_str().to_os_string(),
                pdf_path.as_os_str().to_os_string(),
            ]),
            PDFJS_OUTPUT_MAX_BUFFER_BYTES,
            Some(timeout),
        )
        .map_err(|error| format!("could not extract PDF text from {url}: {error}"))?;
        if output.stdout.len() > PDFJS_OUTPUT_MAX_BUFFER_BYTES
            || output.stderr.len() > PDFJS_OUTPUT_MAX_BUFFER_BYTES
        {
            return Err(format!(
                "PDF.js extraction exceeded the {PDFJS_OUTPUT_MAX_BUFFER_BYTES}-byte output limit"
            ));
        }
        if !output.status.success() {
            let stderr_text = normalize_whitespace(&String::from_utf8_lossy(&output.stderr));
            let stdout_text = normalize_whitespace(&String::from_utf8_lossy(&output.stdout));
            let details = if !stderr_text.is_empty() {
                stderr_text
            } else if !stdout_text.is_empty() {
                stdout_text
            } else {
                format!("PDF.js helper exited with status {}", output.status)
            };
            return Err(format!("could not extract PDF text from {url}: {details}"));
        }

        let pages: Vec<ExtractedPdfPage> = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("could not decode PDF.js text output for {url}: {error}"))?;
        let mut page_blocks = Vec::with_capacity(pages.len());
        for page in pages {
            let line_texts =
                build_pdf_page_line_texts(&PdfTextContent { items: page.items }, course_parser);
            let page_text = normalize_whitespace(&line_texts.join(" "));
            if !page_text.is_empty() {
                page_blocks.push(PdfPageBlock {
                    page_number: page.page_number,
                    page_text,
                    line_texts,
                });
            }
        }
        Ok(page_blocks)
    }
}

fn create_unique_temp_dir(parent: &Path) -> Result<PathBuf, String> {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    for _ in 0..100 {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!("transfer-planner-pdf-{}-{id}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "could not create temporary PDF directory {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Err(format!(
        "could not allocate a unique temporary PDF directory under {}",
        parent.display()
    ))
}

fn normalize_whitespace(value: &str) -> String {
    super::requirement_shape::normalize_js_whitespace(value)
}
