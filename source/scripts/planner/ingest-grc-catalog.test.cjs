const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const {
  buildCourseEntries,
  buildPagedCourseDescriptionUrl,
  detectCatalogPageNumbers,
  validateCatalogResponseBody,
  fetchCatalogHtml,
  formatFailedPagesMarkdown,
} = require("./ingest-grc-catalog.cjs");

const courseDescriptionsUrl =
  "https://catalog.greenriver.edu/content.php?catoid=11&navoid=680";

test("Green River course-description URLs advance to the requested catalog page", () => {
  for (const pageNumber of [10, 11]) {
    const url = new URL(buildPagedCourseDescriptionUrl(courseDescriptionsUrl, pageNumber));
    assert.equal(url.searchParams.get("catoid"), "11");
    assert.equal(url.searchParams.get("navoid"), "680");
    assert.equal(url.searchParams.get("filter[item_type]"), "3");
    assert.equal(url.searchParams.get("filter[only_active]"), "1");
    assert.equal(url.searchParams.get("filter[3]"), "1");
    assert.equal(url.searchParams.get("filter[cpage]"), String(pageNumber));
    assert.equal(url.searchParams.get("expand"), "1");
  }
});

test("pagination detection fills every page between the first and last linked page", () => {
  const pagerHtml = [
    '<a href="?filter%5Bcpage%5D=10">10</a>',
    '<a href="?filter[cpage]=12">12</a>',
  ].join(" ");

  assert.deepEqual(detectCatalogPageNumbers(pagerHtml), Array.from({ length: 12 }, (_, i) => i + 1));
});

test("catalog ingestion rejects HTTP 202 instead of treating it as an empty successful page", () => {
  assert.throws(
    () => validateCatalogResponseBody(courseDescriptionsUrl, 202, ""),
    (error) => error.status === 202 && /202 Accepted/.test(error.message)
  );
});

test("catalog ingestion rejects empty successful responses", () => {
  assert.throws(
    () => validateCatalogResponseBody(courseDescriptionsUrl, 200, "  \n"),
    (error) => error.status === 200 && /empty Green River catalog response/.test(error.message)
  );
});

test("catalog ingestion accepts a non-empty completed catalog page", () => {
  const html = "<html><h3>MATH& 151 - Calculus I</h3></html>";
  assert.equal(validateCatalogResponseBody(courseDescriptionsUrl, 200, html), html);
});

test("course catalog entries extract enrollment prerequisite codes", () => {
  const html =
    "<h3>MATH& 152 - Calculus II</h3>Credits: 5<br><br><strong>Enrollment Prerequisite:</strong> MATH& 151 with a grade of 2.0 or higher.";
  const [course] = buildCourseEntries(html, "2026-2027", courseDescriptionsUrl);

  assert.equal(course.code, "MATH& 152");
  assert.equal(course.catalogDescription, null);
  assert.deepEqual(course.prerequisiteCourseCodes, ["MATH& 151"]);
  assert.match(course.prerequisiteNotes[0], /Official Green River enrollment requirement text/);
});

test("course entries preserve clickable course-code evidence only from enrollment requirements", () => {
  const html = [
    "<h3>MATH& 153 - Calculus III</h3>",
    "Credits: 5",
    '<br>See <a href="preview_course_nopop.php?coid=199">MATH&amp; 199</a> for placement information.',
    "<br><br><strong>Enrollment Requirement:</strong>",
    '<a href="preview_course_nopop.php?coid=151">MATH&amp; 151</a> or',
    '<a href="preview_course_nopop.php?coid=152">MATH&amp; 152</a>.',
  ].join(" ");
  const [course] = buildCourseEntries(html, "2026-2027", courseDescriptionsUrl);
  const clickableEvidence = course.prerequisiteNotes.find((note) =>
    note.startsWith("Clickable catalog course-code references")
  );

  assert.deepEqual(course.prerequisiteAlternativeCourseCodeSets, [
    ["MATH& 151"],
    ["MATH& 152"],
  ]);
  assert.equal(
    clickableEvidence,
    "Clickable catalog course-code references within the enrollment requirement: MATH& 151, MATH& 152."
  );
  assert.doesNotMatch(clickableEvidence, /MATH& 199/);
});

test("unlinked text candidates are retained and flagged when course links are present", () => {
  const html = [
    "<h3>MATH& 153 - Calculus III</h3>",
    "Credits: 5<br><br><strong>Enrollment Requirement:</strong>",
    '<a href="preview_course_nopop.php?coid=151">MATH&amp; 151</a> or MATH&amp; 152.',
  ].join(" ");
  const [course] = buildCourseEntries(html, "2026-2027", courseDescriptionsUrl);

  assert.deepEqual(course.prerequisiteAlternativeCourseCodeSets, [
    ["MATH& 151"],
    ["MATH& 152"],
  ]);
  assert.ok(
    course.prerequisiteNotes.some((note) =>
      note.includes("course codes not linked in this section (retained from official text; review): MATH& 152")
    )
  );
});

test("successful catalog pages do not emit per-page success messages", async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "grc-catalog-page-success-"));
  const snapshotPath = path.join(directory, "catalog.html");
  const messages = [];
  const originalLog = console.log;
  const originalWarn = console.warn;
  console.log = (...values) => messages.push(values.join(" "));
  console.warn = (...values) => messages.push(values.join(" "));
  try {
    const result = await fetchCatalogHtml(courseDescriptionsUrl, snapshotPath, {
      fetchPage: async () => ({
        status: 200,
        html: "<h3>MATH 121 - Precalculus</h3>",
        documentStatuses: [202, 200],
      }),
      detectPageNumbers: () => [1],
      parseEntries: () => [{ code: "MATH 121" }],
    });

    assert.deepEqual(result.failedPages, []);
    assert.equal(result.usedSnapshotFallback, false);
    assert.deepEqual(messages, []);
    assert.equal(fs.existsSync(snapshotPath), true);
  } finally {
    console.log = originalLog;
    console.warn = originalWarn;
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

test("catalog page failures are reported and later pages are still attempted", async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "grc-catalog-page-failure-"));
  const snapshotPath = path.join(directory, "catalog.html");
  const cachedHtml = "<html><h3>CACHED 100 - prior snapshot</h3></html>";
  fs.writeFileSync(snapshotPath, cachedHtml);
  const requestedUrls = [];
  const observedFailures = [];
  const messages = [];
  const originalLog = console.log;
  const originalWarn = console.warn;
  console.log = (...values) => messages.push(values.join(" "));
  console.warn = (...values) => messages.push(values.join(" "));
  try {
    const result = await fetchCatalogHtml(courseDescriptionsUrl, snapshotPath, {
      catalogYearLabel: "2026-2027",
      fetchPage: async (url) => {
        requestedUrls.push(url);
        const page = new URL(url).searchParams.get("filter[cpage]");
        if (page === "2") return { status: 503, html: "temporarily unavailable" };
        return { status: 200, html: `<h3>MATH ${page === "3" ? "123" : "121"} - Course</h3>` };
      },
      detectPageNumbers: () => [1, 2, 3],
      parseEntries: () => [{ code: "MATH 121" }],
      onPageFailure: (failure) => observedFailures.push(failure),
    });

    assert.deepEqual(
      requestedUrls.map((url) => new URL(url).searchParams.get("filter[cpage]") ?? "1"),
      ["1", "2", "3"]
    );
    assert.equal(result.usedSnapshotFallback, true);
    assert.equal(result.html, cachedHtml);
    assert.equal(result.failedPages.length, 1);
    assert.equal(result.failedPages[0].catalogYearLabel, "2026-2027");
    assert.equal(result.failedPages[0].pageNumber, 2);
    assert.equal(result.failedPages[0].status, 503);
    assert.equal(result.failedPages[0].url, requestedUrls[1]);
    assert.deepEqual(observedFailures, result.failedPages);
    assert.match(messages.join("\n"), /Failed Green River catalog page 2026-2027 2 \(HTTP 503\)/);
    assert.doesNotMatch(messages.join("\n"), /catalog pages fetched/);
    assert.match(formatFailedPagesMarkdown(result.failedPages).join("\n"), /page 2 \(HTTP 503\)/);
    assert.deepEqual(formatFailedPagesMarkdown([]), []);
    assert.equal(fs.readFileSync(snapshotPath, "utf8"), cachedHtml);
  } finally {
    console.log = originalLog;
    console.warn = originalWarn;
    fs.rmSync(directory, { recursive: true, force: true });
  }
});
