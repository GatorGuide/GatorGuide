use super::requirement_html::{RequirementHtmlArtifacts, parse_html_artifacts};
use super::source_artifact_cache::{BoundedResultCache, DEFAULT_SOURCE_ARTIFACT_CACHE_MAX_ENTRIES};
use super::requirement_source_fetching::SourceDownloader;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct HtmlSourceParser {
    downloader: SourceDownloader,
    cache: Arc<BoundedResultCache<RequirementHtmlArtifacts>>,
}

impl HtmlSourceParser {
    pub fn new(downloader: SourceDownloader) -> Self {
        Self::with_cache_capacity(downloader, DEFAULT_SOURCE_ARTIFACT_CACHE_MAX_ENTRIES)
    }

    pub fn with_cache_capacity(downloader: SourceDownloader, max_cache_entries: usize) -> Self {
        Self {
            downloader,
            cache: Arc::new(BoundedResultCache::new(max_cache_entries)),
        }
    }

    pub fn get_html_source_artifacts(
        &self,
        url: &str,
        timeout: Duration,
    ) -> Result<RequirementHtmlArtifacts, String> {
        self.cache.get_or_load(url, || {
            let downloaded = self.downloader.download_source(url, timeout, false)?;
            let html = String::from_utf8_lossy(&downloaded.body);
            Ok(parse_html_artifacts(&html))
        })
    }
}
