const test = require("node:test");
const assert = require("node:assert/strict");
const {
  createGrcBrowserFetcher,
  findChromiumExecutable,
  samePageUrl,
} = require("./grc-browser-fetch.cjs");

const pageUrl = "https://catalog.greenriver.edu/content.php?catoid=11&navoid=680&expand=1";

function makeResponse(status, url = pageUrl) {
  return {
    request: () => ({ resourceType: () => "document" }),
    status: () => status,
    url: () => url,
    headerValue: async () => null,
  };
}

function makeChromium({ responses, state }) {
  let launchOptions;
  let closed = false;
  const listeners = new Map();
  const page = {
    on: (eventName, listener) => listeners.set(eventName, listener),
    removeListener: (eventName) => listeners.delete(eventName),
    goto: async () => {
      for (const response of responses) listeners.get("response")?.(response);
      return responses[0] ?? null;
    },
    evaluate: async () => state,
    waitForTimeout: async () => {},
    content: async () => "<html><body><h3>MATH&amp; 151 - Calculus I</h3></body></html>",
  };
  const context = {
    newPage: async () => page,
    close: async () => { closed = true; },
  };
  const browser = {
    newContext: async () => context,
    close: async () => { closed = true; },
  };
  return {
    chromium: {
      launch: async (options) => {
        launchOptions = options;
        return browser;
      },
    },
    getLaunchOptions: () => launchOptions,
    isClosed: () => closed,
  };
}

function readyState(overrides = {}) {
  return {
    url: pageUrl,
    title: "Course Descriptions - Green River College",
    readyState: "complete",
    bodyText: "MATH& 151 - Calculus I. Enrollment prerequisite: MATH& 141.",
    bodyTextLength: 63,
    htmlLength: 1800,
    courseHeadingCount: 1,
    ...overrides,
  };
}

test("Chromium executable override is honored and checked", () => {
  assert.throws(
    () => findChromiumExecutable({
      env: { GATORGUIDE_CHROMIUM_EXECUTABLE: "missing/chrome.exe" },
      exists: () => false,
    }),
    /GATORGUIDE_CHROMIUM_EXECUTABLE does not exist/
  );
  const found = findChromiumExecutable({
    env: { GATORGUIDE_CHROMIUM_EXECUTABLE: "configured/chrome.exe" },
    exists: () => true,
  });
  assert.equal(found, require("node:path").resolve("configured/chrome.exe"));
});

test("catalog browser waits for a completed rendered page after the initial HTTP 202", async () => {
  const fake = makeChromium({
    responses: [makeResponse(202), makeResponse(200)],
    state: readyState(),
  });
  const fetcher = await createGrcBrowserFetcher({
    chromium: fake.chromium,
    executablePath: "configured-chrome.exe",
    stablePolls: 2,
    pollIntervalMs: 0,
  });
  try {
    const result = await fetcher.fetchPage(pageUrl, 100, { expectCourseListings: true });
    assert.equal(result.status, 200);
    assert.deepEqual(result.documentStatuses, [202, 200]);
    assert.match(result.html, /MATH&amp; 151/);
    assert.equal(fake.getLaunchOptions().headless, false);
    assert.ok(fake.getLaunchOptions().args.includes("--start-minimized"));
  } finally {
    await fetcher.close();
  }
  assert.equal(fake.isClosed(), true);
});

test("catalog browser rejects an unresolved HTTP 202 instead of parsing it", async () => {
  const fake = makeChromium({
    responses: [makeResponse(202)],
    state: readyState({ bodyText: "Request accepted; processing.", courseHeadingCount: 0 }),
  });
  const fetcher = await createGrcBrowserFetcher({
    chromium: fake.chromium,
    executablePath: "configured-chrome.exe",
    readyTimeoutMs: 5,
    pollIntervalMs: 1,
    stablePolls: 1,
  });
  try {
    await assert.rejects(
      fetcher.fetchPage(pageUrl, 5, { expectCourseListings: true }),
      (error) => error.status === 202 && /no completed Green River page/.test(error.message)
    );
  } finally {
    await fetcher.close();
  }
});

test("catalog browser refuses a human-verification page", async () => {
  const fake = makeChromium({
    responses: [makeResponse(200)],
    state: readyState({ title: "Verify you are human", courseHeadingCount: 0 }),
  });
  const fetcher = await createGrcBrowserFetcher({
    chromium: fake.chromium,
    executablePath: "configured-chrome.exe",
  });
  try {
    await assert.rejects(
      fetcher.fetchPage(pageUrl, 100),
      /human-verification or access challenge/
    );
  } finally {
    await fetcher.close();
  }
});

test("same catalog page comparison ignores query parameter encoding and order", () => {
  assert.equal(
    samePageUrl(pageUrl, "https://catalog.greenriver.edu/content.php?expand=1&navoid=680&catoid=11"),
    true
  );
});
