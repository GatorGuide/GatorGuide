const fs = require("node:fs");
const path = require("node:path");

const DEFAULT_READY_TIMEOUT_MS = 45000;
const DEFAULT_POLL_INTERVAL_MS = 250;
const DEFAULT_STABLE_POLLS = 4;
const CHALLENGE_PATTERN = /\b(?:captcha|human verification|verify (?:that )?you are human|checking your browser|access denied|automated requests)\b/i;

function findChromiumExecutable(options = {}) {
  const env = options.env ?? process.env;
  const platform = options.platform ?? process.platform;
  const exists = options.exists ?? fs.existsSync;
  const explicit = String(env.GATORGUIDE_CHROMIUM_EXECUTABLE ?? "").trim();
  if (explicit) {
    const resolved = path.resolve(explicit);
    if (!exists(resolved)) {
      throw new Error(`GATORGUIDE_CHROMIUM_EXECUTABLE does not exist: ${resolved}`);
    }
    return resolved;
  }

  const candidates = [];
  const add = (root, relative) => {
    if (root) candidates.push(path.join(root, relative));
  };
  if (platform === "win32") {
    for (const root of [env.LOCALAPPDATA, env.PROGRAMFILES, env["ProgramFiles(x86)"], env.PROGRAMW6432]) {
      add(root, "Chromium/Application/chrome.exe");
      add(root, "Google/Chrome/Application/chrome.exe");
    }
  } else if (platform === "darwin") {
    candidates.push(
      "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
      "/Applications/Chromium.app/Contents/MacOS/Chromium",
      path.join(env.HOME ?? "", "Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
      path.join(env.HOME ?? "", "Applications/Chromium.app/Contents/MacOS/Chromium")
    );
  }

  for (const directory of String(env.PATH ?? "").split(path.delimiter).filter(Boolean)) {
    for (const executable of platform === "win32"
      ? ["chrome.exe", "chromium.exe"]
      : ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser"])
      candidates.push(path.join(directory, executable));
  }

  if (options.bundledExecutablePath) candidates.push(options.bundledExecutablePath);
  const found = candidates.find((candidate) => exists(candidate));
  return found ? path.resolve(found) : null;
}

function comparablePageUrl(value) {
  const url = new URL(value);
  const search = [...url.searchParams.entries()]
    .sort(([leftKey, leftValue], [rightKey, rightValue]) =>
      leftKey.localeCompare(rightKey) || leftValue.localeCompare(rightValue)
    )
    .map(([key, entryValue]) => `${key}=${entryValue}`)
    .join("&");
  return `${url.origin}${url.pathname}?${search}`;
}

function samePageUrl(left, right) {
  try {
    return comparablePageUrl(left) === comparablePageUrl(right);
  } catch {
    return false;
  }
}

function createBrowserFetchError(url, message, status = null, retryAfter = null, statuses = []) {
  const error = new Error(message);
  error.url = url;
  error.status = status;
  error.retryAfter = retryAfter;
  error.documentStatuses = statuses;
  return error;
}

async function responseRetryAfter(response) {
  try {
    return await response.headerValue("retry-after");
  } catch {
    return null;
  }
}

async function createGrcBrowserFetcher(options = {}) {
  let chromium = options.chromium;
  if (!chromium) {
    try {
      ({ chromium } = require("playwright"));
    } catch (error) {
      throw new Error(`Playwright is required for browser-based Green River catalog capture: ${error.message}`);
    }
  }

  const bundledExecutablePath = options.bundledExecutablePath ?? chromium.executablePath?.();
  const executablePath = options.executablePath ?? findChromiumExecutable({
    env: options.env,
    platform: options.platform,
    exists: options.exists,
    bundledExecutablePath,
  });
  if (!executablePath) {
    throw new Error(
      "Chrome/Chromium was not found. Set GATORGUIDE_CHROMIUM_EXECUTABLE or install Playwright Chromium; the catalog downloader will not fall back to raw HTTP."
    );
  }

  let browser;
  try {
    browser = await chromium.launch({
      executablePath,
      headless: false,
      args: ["--start-minimized"],
    });
  } catch (error) {
    throw new Error(`Could not launch minimized Chrome/Chromium for catalog capture: ${error.message}`);
  }

  let context;
  try {
    context = await browser.newContext({
      viewport: { width: 1365, height: 900 },
      acceptDownloads: false,
    });
    const page = await context.newPage();
    const readyTimeoutMs = options.readyTimeoutMs ?? DEFAULT_READY_TIMEOUT_MS;
    const pollIntervalMs = options.pollIntervalMs ?? DEFAULT_POLL_INTERVAL_MS;
    const stablePolls = options.stablePolls ?? DEFAULT_STABLE_POLLS;
    let busy = false;
    let closed = false;

    return {
      async fetchPage(url, timeoutMs = readyTimeoutMs, fetchOptions = {}) {
        if (closed) throw new Error("The Green River Chromium fetcher is already closed.");
        if (busy) throw new Error("Green River browser pages must be fetched sequentially.");
        busy = true;
        const documentResponses = [];
        const onResponse = (response) => {
          try {
            if (response.request().resourceType() !== "document") return;
            documentResponses.push({
              response,
              status: response.status(),
              url: response.url(),
            });
          } catch {
            // Ignore responses that disappear while Chromium changes documents.
          }
        };
        page.on("response", onResponse);
        let navigationError = null;
        try {
          try {
            await page.goto(url, { waitUntil: "domcontentloaded", timeout: timeoutMs });
          } catch (error) {
            navigationError = error;
            if (documentResponses.length === 0) {
              throw createBrowserFetchError(
                url,
                `Chrome/Chromium could not navigate to the Green River page: ${error.message}`
              );
            }
          }

          const deadline = Date.now() + timeoutMs;
          let priorSignature = null;
          let stableCount = 0;
          let lastState = null;
          let matchingResponses = [];
          while (Date.now() < deadline) {
            try {
              lastState = await page.evaluate(() => {
                const bodyText = document.body?.innerText ?? "";
                const html = document.documentElement?.outerHTML ?? "";
                return {
                  url: location.href,
                  title: document.title ?? "",
                  readyState: document.readyState,
                  bodyText,
                  bodyTextLength: bodyText.length,
                  htmlLength: html.length,
                  courseHeadingCount: document.querySelectorAll("h3").length,
                };
              });
            } catch {
              await page.waitForTimeout(pollIntervalMs);
              continue;
            }

            matchingResponses = documentResponses.filter((item) => samePageUrl(item.url, lastState.url));
            const finalDocument = matchingResponses.at(-1);
            const statuses = matchingResponses.map((item) => item.status);
            if (CHALLENGE_PATTERN.test(`${lastState.title}\n${lastState.bodyText.slice(0, 3000)}`)) {
              throw createBrowserFetchError(
                url,
                "Chrome/Chromium reached a human-verification or access challenge; capture stopped without saving the page.",
                finalDocument?.status ?? null,
                await responseRetryAfter(finalDocument?.response),
                statuses
              );
            }
            if (finalDocument && finalDocument.status >= 400) {
              throw createBrowserFetchError(
                url,
                `Chrome/Chromium received HTTP ${finalDocument.status} for the Green River page.`,
                finalDocument.status,
                await responseRetryAfter(finalDocument.response),
                statuses
              );
            }

            const completedStatus =
              finalDocument &&
              ((finalDocument.status >= 200 && finalDocument.status < 300 && finalDocument.status !== 202) ||
                finalDocument.status === 304);
            const hasExpectedContent =
              lastState.bodyTextLength > 0 &&
              lastState.htmlLength > 1000 &&
              (!fetchOptions.expectCourseListings || lastState.courseHeadingCount > 0);
            if (completedStatus && lastState.readyState === "complete" && hasExpectedContent) {
              const signature = [
                lastState.url,
                lastState.title,
                lastState.bodyTextLength,
                lastState.htmlLength,
                lastState.courseHeadingCount,
              ].join("|");
              stableCount = signature === priorSignature ? stableCount + 1 : 1;
              priorSignature = signature;
              if (stableCount >= stablePolls) {
                const html = await page.content();
                if (!html.trim()) {
                  throw createBrowserFetchError(url, "Chrome/Chromium returned an empty rendered Green River page.", finalDocument.status, null, statuses);
                }
                return {
                  status: finalDocument.status,
                  html,
                  title: lastState.title,
                  url: lastState.url,
                  bodyTextLength: lastState.bodyTextLength,
                  courseHeadingCount: lastState.courseHeadingCount,
                  documentStatuses: statuses,
                  retryAfter: await responseRetryAfter(finalDocument.response),
                };
              }
            } else {
              stableCount = 0;
              priorSignature = null;
            }
            await page.waitForTimeout(pollIntervalMs);
          }

          const finalDocument = matchingResponses.at(-1);
          const statuses = matchingResponses.map((item) => item.status);
          if (finalDocument?.status === 202) {
            throw createBrowserFetchError(
              url,
              "Chrome/Chromium received HTTP 202 Accepted but no completed Green River page arrived before the readiness timeout.",
              202,
              await responseRetryAfter(finalDocument.response),
              statuses
            );
          }
          const detail = navigationError ? ` Navigation detail: ${navigationError.message}` : "";
          throw createBrowserFetchError(
            url,
            `Chrome/Chromium did not reach stable Green River page content before the readiness timeout.${detail}`,
            finalDocument?.status ?? null,
            await responseRetryAfter(finalDocument?.response),
            statuses
          );
        } finally {
          page.removeListener("response", onResponse);
          busy = false;
        }
      },
      async close() {
        if (closed) return;
        closed = true;
        try {
          await context.close();
        } finally {
          await browser.close();
        }
      },
    };
  } catch (error) {
    await browser.close();
    throw error;
  }
}

module.exports = {
  createGrcBrowserFetcher,
  findChromiumExecutable,
  samePageUrl,
};
