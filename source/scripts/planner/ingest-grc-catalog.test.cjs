const test = require("node:test");
const assert = require("node:assert/strict");
const {
  buildCourseEntries,
  buildPagedCourseDescriptionUrl,
  detectCatalogPageNumbers,
  validateCatalogResponseBody,
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
    "<h3>MATH& 152 - Calculus II</h3>Credits: 5<br><br><strong>Enrollment Requirement:</strong> MATH& 151 with a grade of 2.0 or higher.";
  const [course] = buildCourseEntries(html, "2026-2027", courseDescriptionsUrl);

  assert.equal(course.code, "MATH& 152");
  assert.deepEqual(course.prerequisiteCourseCodes, ["MATH& 151"]);
  assert.match(course.prerequisiteNotes[0], /Official Green River enrollment requirement text/);
});
