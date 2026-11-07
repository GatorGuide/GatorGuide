use reqwest::blocking::Client;
use reqwest::header::{RETRY_AFTER, USER_AGENT};
use std::collections::HashMap;
use std::io::{self, Read};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use url::Url;

pub const DEFAULT_RETRY_ATTEMPTS: usize = 3;
pub const DEFAULT_HOST_COOLDOWN_MS: u64 = 900;
pub const DEFAULT_MAX_RETRY_DELAY_MS: u64 = 60_000;
pub const DEFAULT_USER_AGENT: &str = "GatorGuideTransferPlannerRequirementParser/1.0";
#[cfg(windows)]
pub const DEFAULT_CURL_COMMAND: &str = "curl.exe";
#[cfg(not(windows))]
pub const DEFAULT_CURL_COMMAND: &str = "curl";
pub const DEFAULT_CURL_ACCEPT_HEADER: &str =
    "text/html,application/xhtml+xml,application/xml;q=0.9,application/pdf;q=0.8,*/*;q=0.7";
pub const SOURCE_DOWNLOAD_MAX_BUFFER_BYTES: usize = 40 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFetchMode {
    Fetch,
    Curl,
}

impl SourceFetchMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::Curl => "curl",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedSource {
    pub body: Vec<u8>,
    pub fetch_mode: SourceFetchMode,
}

impl DownloadedSource {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

#[derive(Debug, Clone)]
pub struct SourceDownloaderOptions {
    pub retry_attempts: usize,
    pub host_cooldown_ms: u64,
    pub max_retry_delay_ms: u64,
    pub user_agent: String,
    pub curl_command: String,
    pub curl_accept_header: String,
    pub max_buffer_bytes: usize,
    pub scope_min_delay_ms: u64,
}

impl Default for SourceDownloaderOptions {
    fn default() -> Self {
        Self {
            retry_attempts: DEFAULT_RETRY_ATTEMPTS,
            host_cooldown_ms: DEFAULT_HOST_COOLDOWN_MS,
            max_retry_delay_ms: DEFAULT_MAX_RETRY_DELAY_MS,
            user_agent: DEFAULT_USER_AGENT.to_string(),
            curl_command: DEFAULT_CURL_COMMAND.to_string(),
            curl_accept_header: DEFAULT_CURL_ACCEPT_HEADER.to_string(),
            max_buffer_bytes: SOURCE_DOWNLOAD_MAX_BUFFER_BYTES,
            scope_min_delay_ms: 1_500,
        }
    }
}

#[derive(Clone)]
pub struct SourceDownloader {
    inner: Arc<DownloaderInner>,
}

struct DownloaderInner {
    options: SourceDownloaderOptions,
    client: Client,
    cache: Mutex<HashMap<String, Arc<OnceLock<Result<DownloadedSource, String>>>>>,
    limiter: HostRateLimiter,
}

#[derive(Default)]
struct HostRateLimiter {
    scopes: Mutex<HashMap<String, Arc<Mutex<ScopeState>>>>,
}

struct ScopeState {
    next_allowed_at: Instant,
    penalty_ms: u64,
}

impl Default for ScopeState {
    fn default() -> Self {
        Self {
            next_allowed_at: Instant::now(),
            penalty_ms: 0,
        }
    }
}

#[derive(Debug)]
struct FetchResponse {
    status: u16,
    status_text: String,
    retry_after: Option<String>,
    body: Vec<u8>,
}

impl SourceDownloader {
    pub fn new(options: SourceDownloaderOptions) -> Result<Self, String> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(|error| format!("could not create requirement-source HTTP client: {error}"))?;
        Ok(Self {
            inner: Arc::new(DownloaderInner {
                limiter: HostRateLimiter::default(),
                cache: Mutex::new(HashMap::new()),
                client,
                options,
            }),
        })
    }

    pub fn download_source(
        &self,
        url: &str,
        timeout: Duration,
        binary: bool,
    ) -> Result<DownloadedSource, String> {
        let sanitized_url = sanitize_source_url(url);
        let cache_key = format!("{}::{sanitized_url}", if binary { "binary" } else { "text" });
        let cached = {
            let mut cache = self
                .inner
                .cache
                .lock()
                .map_err(|_| "requirement-source download cache was poisoned".to_string())?;
            cache
                .entry(cache_key.clone())
                .or_insert_with(|| Arc::new(OnceLock::new()))
                .clone()
        };
        let result = cached
            .get_or_init(|| self.download_uncached(&sanitized_url, timeout, binary))
            .clone();
        if result.is_err() {
            if let Ok(mut cache) = self.inner.cache.lock() {
                if cache
                    .get(&cache_key)
                    .is_some_and(|current| Arc::ptr_eq(current, &cached))
                {
                    cache.remove(&cache_key);
                }
            }
        }
        result
    }

    pub fn fetch_with_retries(
        &self,
        url: &str,
        timeout: Duration,
    ) -> Result<(u16, String, Vec<u8>), String> {
        self.fetch_with_retries_inner(url, timeout)
            .map(|response| (response.status, response.status_text, response.body))
    }

    fn download_uncached(
        &self,
        url: &str,
        timeout: Duration,
        binary: bool,
    ) -> Result<DownloadedSource, String> {
        let mut fetch_response = None;
        let mut fetch_error = None;
        match self.fetch_with_retries_inner(url, timeout) {
            Ok(response) if (200..300).contains(&response.status) => {
                return Ok(DownloadedSource {
                    body: response.body,
                    fetch_mode: SourceFetchMode::Fetch,
                });
            }
            Ok(response) => fetch_response = Some(response),
            Err(error) => fetch_error = Some(error),
        }

        match self.download_with_curl(url, timeout, binary) {
            Ok(download) => Ok(download),
            Err(curl_error) => {
                if let Some(response) = fetch_response {
                    Err(format!("HTTP {} {}", response.status, response.status_text))
                } else if let Some(error) = fetch_error {
                    Err(error)
                } else {
                    Err(curl_error)
                }
            }
        }
    }

    fn fetch_with_retries_inner(
        &self,
        url: &str,
        timeout: Duration,
    ) -> Result<FetchResponse, String> {
        let attempts = self.inner.options.retry_attempts;
        let mut last_response = None;
        let mut last_error = None;
        for attempt in 1..=attempts {
            let response = self.inner.limiter.with_throttle(url, &self.inner.options, || {
                self.fetch_once(url, timeout)
            });
            match response {
                Ok(response) if (200..300).contains(&response.status) => {
                    self.inner.limiter.clear_rate_limit_penalty(url);
                    return Ok(response);
                }
                Ok(response) => {
                    let retryable = is_retryable_http_status(response.status);
                    let delay = get_retry_delay_ms(
                        attempt,
                        response.retry_after.as_deref(),
                        self.inner.options.host_cooldown_ms,
                        self.inner.options.max_retry_delay_ms,
                    );
                    if retryable && attempt < attempts {
                        if matches!(response.status, 429 | 503) {
                            self.inner.limiter.note_rate_limit(
                                url,
                                delay,
                                &self.inner.options,
                            );
                        }
                        last_response = Some(response);
                        thread::sleep(Duration::from_millis(delay));
                    } else {
                        return Ok(response);
                    }
                }
                Err(error) => {
                    last_error = Some(error);
                    if attempt < attempts {
                        let delay = get_retry_delay_ms(
                            attempt,
                            None,
                            self.inner.options.host_cooldown_ms,
                            self.inner.options.max_retry_delay_ms,
                        );
                        thread::sleep(Duration::from_millis(delay));
                    }
                }
            }
        }
        if let Some(response) = last_response {
            return Ok(response);
        }
        Err(last_error.unwrap_or_else(|| "requirement-source download failed".to_string()))
    }

    fn fetch_once(&self, url: &str, timeout: Duration) -> Result<FetchResponse, String> {
        let response = self
            .inner
            .client
            .get(url)
            .timeout(timeout)
            .header(USER_AGENT, self.inner.options.user_agent.as_str())
            .send()
            .map_err(|error| error.to_string())?;
        let status = response.status();
        let status_text = status.canonical_reason().unwrap_or("unknown status").to_string();
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let body = response
            .bytes()
            .map_err(|error| error.to_string())?
            .to_vec();
        Ok(FetchResponse {
            status: status.as_u16(),
            status_text,
            retry_after,
            body,
        })
    }

    fn download_with_curl(
        &self,
        url: &str,
        timeout: Duration,
        _binary: bool,
    ) -> Result<DownloadedSource, String> {
        let timeout_seconds = timeout
            .as_secs()
            .saturating_add(if timeout.subsec_nanos() > 0 { 1 } else { 0 })
            .max(5);
        let args = [
            "--silent".to_string(),
            "--show-error".to_string(),
            "--location".to_string(),
            "--fail".to_string(),
            "--user-agent".to_string(),
            self.inner.options.user_agent.clone(),
            "--header".to_string(),
            format!("Accept: {}", self.inner.options.curl_accept_header),
            "--max-time".to_string(),
            timeout_seconds.to_string(),
            url.to_string(),
        ];
        let output = self.inner.limiter.with_throttle(url, &self.inner.options, || {
            run_command_bounded(
                Command::new(&self.inner.options.curl_command).args(args),
                self.inner.options.max_buffer_bytes,
            )
        });
        let output = output.map_err(|error| format!("curl failed for {url}: {error}"))?;
        if output.stdout.len() > self.inner.options.max_buffer_bytes
            || output.stderr.len() > self.inner.options.max_buffer_bytes
        {
            return Err(format!("curl output exceeded the {}-byte buffer limit", self.inner.options.max_buffer_bytes));
        }
        if !output.status.success() {
            let details = normalize_whitespace(&String::from_utf8_lossy(&output.stderr))
                .or_else(|| normalize_whitespace(&String::from_utf8_lossy(&output.stdout)))
                .unwrap_or_else(|| format!("curl exited with status {}", output.status));
            return Err(details);
        }
        self.inner.limiter.clear_rate_limit_penalty(url);
        Ok(DownloadedSource {
            body: output.stdout,
            fetch_mode: SourceFetchMode::Curl,
        })
    }
}

impl HostRateLimiter {
    fn scope_state(&self, url: &str) -> Arc<Mutex<ScopeState>> {
        let scope = get_rate_limit_scope_key(url);
        let mut scopes = self.scopes.lock().unwrap_or_else(|error| error.into_inner());
        scopes
            .entry(scope)
            .or_insert_with(|| Arc::new(Mutex::new(ScopeState::default())))
            .clone()
    }

    fn with_throttle<T>(
        &self,
        url: &str,
        options: &SourceDownloaderOptions,
        work: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let state = self.scope_state(url);
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        let remaining = state.next_allowed_at.saturating_duration_since(Instant::now());
        if !remaining.is_zero() {
            thread::sleep(remaining);
        }
        let result = work();
        let min_delay = resolve_min_delay_ms(&get_rate_limit_scope_key(url), state.penalty_ms, options);
        state.next_allowed_at = Instant::now() + Duration::from_millis(min_delay);
        result
    }

    fn note_rate_limit(&self, url: &str, delay_ms: u64, options: &SourceDownloaderOptions) {
        let state = self.scope_state(url);
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        let bounded = delay_ms.min(options.max_retry_delay_ms);
        state.next_allowed_at = state
            .next_allowed_at
            .max(Instant::now() + Duration::from_millis(bounded));
        let minimum = resolve_min_delay_ms(&get_rate_limit_scope_key(url), state.penalty_ms, options);
        state.penalty_ms = options
            .max_retry_delay_ms
            .min((minimum.saturating_mul(2)).max(bounded));
    }

    fn clear_rate_limit_penalty(&self, url: &str) {
        let state = self.scope_state(url);
        state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .penalty_ms = 0;
    }
}

pub fn build_curl_error_message(stderr: &str, stdout: &str, message: &str, url: &str) -> String {
    normalize_whitespace(stderr)
        .or_else(|| normalize_whitespace(stdout))
        .or_else(|| normalize_whitespace(message))
        .unwrap_or_else(|| format!("curl failed for {url}"))
}

pub fn sanitize_source_url(value: &str) -> String {
    let trimmed = value.trim();
    trimmed.strip_suffix('?').unwrap_or(trimmed).to_string()
}

pub fn is_retryable_http_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

pub fn get_host_key(value: &str) -> String {
    Url::parse(value)
        .ok()
        .and_then(|url| {
            let host = url.host_str()?.to_ascii_lowercase();
            Some(match url.port() {
                Some(port) => format!("{host}:{port}"),
                None => host,
            })
        })
        .unwrap_or_else(|| "unknown-host".to_string())
}

pub fn get_rate_limit_scope_key(url_or_host: &str) -> String {
    let host = if url_or_host.contains("://") {
        get_host_key(url_or_host)
    } else {
        url_or_host.to_ascii_lowercase()
    };
    if host.is_empty() || host == "unknown-host" {
        return "scope:unknown-host".to_string();
    }
    if host == "washington.edu" || host.ends_with(".washington.edu") {
        return "scope:washington.edu".to_string();
    }
    if ["uw.edu", "uwb.edu"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
    {
        return "scope:uw.edu".to_string();
    }
    if host == "greenriver.edu" || host.ends_with(".greenriver.edu") {
        return "scope:greenriver.edu".to_string();
    }
    format!("host:{host}")
}

pub fn parse_retry_after_to_ms(value: Option<&str>) -> Option<u64> {
    let normalized = normalize_whitespace(value.unwrap_or_default());
    if normalized.is_empty() {
        return None;
    }
    if normalized.chars().all(|character| character.is_ascii_digit()) {
        return normalized.parse::<u64>().ok()?.checked_mul(1_000);
    }
    let retry_at = chrono::DateTime::parse_from_rfc2822(&normalized).ok()?;
    let delay = retry_at.timestamp_millis() - chrono::Utc::now().timestamp_millis();
    Some(delay.max(0) as u64)
}

fn get_retry_delay_ms(
    attempt: usize,
    retry_after: Option<&str>,
    host_cooldown_ms: u64,
    max_retry_delay_ms: u64,
) -> u64 {
    let exponential = host_cooldown_ms
        .saturating_mul(1u64.checked_shl(attempt.saturating_sub(1).min(63) as u32).unwrap_or(u64::MAX))
        .max(host_cooldown_ms)
        .min(max_retry_delay_ms);
    parse_retry_after_to_ms(retry_after)
        .map(|retry_after| retry_after.max(exponential).max(host_cooldown_ms).min(max_retry_delay_ms))
        .unwrap_or(exponential)
}

fn resolve_min_delay_ms(
    scope_key: &str,
    penalty_ms: u64,
    options: &SourceDownloaderOptions,
) -> u64 {
    let scoped_floor = if scope_key.starts_with("scope:") {
        options.scope_min_delay_ms
    } else {
        options.host_cooldown_ms
    };
    options.host_cooldown_ms.max(scoped_floor).max(penalty_ms)
}

fn run_command_bounded(command: &mut Command, max_buffer_bytes: usize) -> Result<Output, String> {
    run_command_bounded_with_timeout(command, max_buffer_bytes, None)
}

pub(crate) fn run_command_bounded_with_timeout(
    command: &mut Command,
    max_buffer_bytes: usize,
    timeout: Option<Duration>,
) -> Result<Output, String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "command stdout pipe was unavailable".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "command stderr pipe was unavailable".to_string())?;
    thread::scope(|scope| {
        let stdout_reader = scope.spawn(move || read_bounded_output(stdout, max_buffer_bytes));
        let stderr_reader = scope.spawn(move || read_bounded_output(stderr, max_buffer_bytes));
        let deadline = timeout.map(|duration| Instant::now() + duration);
        let mut timed_out = false;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => {
                    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                        timed_out = true;
                        let _ = child.kill();
                        break child.wait().map_err(|error| error.to_string());
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(error.to_string());
                }
            }
        };
        let stdout = stdout_reader
            .join()
            .map_err(|_| "command stdout reader failed".to_string())?
            .map_err(|error| error.to_string())?;
        let stderr = stderr_reader
            .join()
            .map_err(|_| "command stderr reader failed".to_string())?
            .map_err(|error| error.to_string())?;
        if timed_out {
            return Err(format!("command timed out after {} ms", timeout.unwrap_or_default().as_millis()));
        }
        Ok(Output {
            status: status?,
            stdout,
            stderr,
        })
    })
}

fn read_bounded_output(mut reader: impl Read, max_bytes: usize) -> io::Result<Vec<u8>> {
    let capture_limit = max_bytes.saturating_add(1);
    let mut captured = Vec::with_capacity(capture_limit.min(8_192));
    let mut buffer = [0u8; 8_192];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = capture_limit.saturating_sub(captured.len());
        captured.extend_from_slice(&buffer[..read.min(remaining)]);
    }
    Ok(captured)
}

fn normalize_whitespace(value: &str) -> Option<String> {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    (!normalized.is_empty()).then_some(normalized)
}
