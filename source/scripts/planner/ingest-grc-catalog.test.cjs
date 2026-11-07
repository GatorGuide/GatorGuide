const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const {
  classifyGrcGeneralEducationRequirementTags,
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

test("Rust catalog sections keep course details aligned when non-course headings are skipped", () => {
  const html = [
    "<h3>MATH& 151 - Calculus I</h3>Credits: 5<br>First course description.",
    "<h3>Admissions - Instructions</h3>Credits: 99<br>Not a course entry.",
    "<h3>ENGL&amp; 101 - Composition</h3>Credits: 2.5<br>Second course description.",
  ].join("");
  const courses = buildCourseEntries(html, "2026-2027", courseDescriptionsUrl);

  assert.deepEqual(
    courses.map(({ code, creditLabel, catalogDescription }) => ({
      code,
      creditLabel,
      catalogDescription,
    })),
    [
      {
        code: "ENGL& 101",
        creditLabel: "2.5",
        catalogDescription: "Second course description.",
      },
      {
        code: "MATH& 151",
        creditLabel: "5",
        catalogDescription: "First course description.",
      },
    ]
  );
});

test("Rust heading candidates retain the GRC legacy course-code alias", () => {
  const html = "<h3>MATH& 254 - Differential Equations</h3>Credits: 5";
  const [course] = buildCourseEntries(html, "2026-2027", courseDescriptionsUrl);

  assert.equal(course.code, "MATH& 264");
  assert.equal(course.title, "Differential Equations");
});

test("course catalog entries extract enrollment prerequisite codes", () => {
  const html =
    "<h3>MATH& 152 - Calculus II</h3>Credits: 5<br><br><strong>Enrollment Prerequisite:</strong> MATH& 151 with a grade of 2.0 or higher.";
  const [course] = buildCourseEntries(html, "2026-2027", courseDescriptionsUrl);

  assert.equal(course.code, "MATH& 152");
  assert.equal(course.creditLabel, "5");
  assert.equal(course.creditValue, 5);
  assert.equal(course.catalogDescription, null);
  assert.deepEqual(course.prerequisiteCourseCodes, ["MATH& 151"]);
  assert.match(course.prerequisiteNotes[0], /Official Green River enrollment requirement text/);
});

test("Rust enrollment parsing materializes simple alternatives with legacy ordering", () => {
  const html = [
    "<h3>CHEM 113 - General Chemistry III</h3>",
    "<br><br><strong>Enrollment Requirement:</strong> MATH 121 or ENGL 101",
  ].join("");
  const [course] = buildCourseEntries(html, "2026-2027", courseDescriptionsUrl);

  assert.deepEqual(course.prerequisiteCourseCodes, []);
  assert.deepEqual(course.prerequisiteAlternativeCourseCodeSets, [
    ["ENGL 101"],
    ["MATH 121"],
  ]);
});

test("Rust enrollment parsing accepts a balanced parenthesized simple expression", () => {
  const html = [
    "<h3>CHEM 114 - General Chemistry IV</h3>",
    "<br><br><strong>Enrollment Requirement:</strong> (MATH 121 or ENGL 101)",
  ].join("");
  const [course] = buildCourseEntries(html, "2026-2027", courseDescriptionsUrl);

  assert.deepEqual(course.prerequisiteCourseCodes, []);
  assert.deepEqual(course.prerequisiteAlternativeCourseCodeSets, [
    ["ENGL 101"],
    ["MATH 121"],
  ]);
});

test("Rust enrollment parsing preserves conjunctions across simple catalog clauses", () => {
  const html = [
    "<h3>CHEM 114 - General Chemistry IV</h3>",
    "<br><br><strong>Enrollment Requirement:</strong> MATH 121; ENGL 101",
  ].join("");
  const [course] = buildCourseEntries(html, "2026-2027", courseDescriptionsUrl);

  assert.deepEqual(course.prerequisiteCourseCodes, ["ENGL 101", "MATH 121"]);
  assert.deepEqual(course.prerequisiteAlternativeCourseCodeSets, []);
});

test("Rust enrollment parsing accepts terminal catalog punctuation", () => {
  const html = [
    "<h3>CHEM 115 - General Chemistry</h3>",
    "<br><br><strong>Enrollment Requirement:</strong> MATH 121.",
  ].join("");
  const [course] = buildCourseEntries(html, "2026-2027", courseDescriptionsUrl);

  assert.deepEqual(course.prerequisiteCourseCodes, ["MATH 121"]);
  assert.deepEqual(course.prerequisiteAlternativeCourseCodeSets, []);
});

test("Rust enrollment parsing preserves symbolic course conjunctions", () => {
  const html = [
    "<h3>CHEM 115 - General Chemistry</h3>",
    "<br><br><strong>Enrollment Requirement:</strong> MATH 121 &amp; ENGL 101 + CHEM 111",
  ].join("");
  const [course] = buildCourseEntries(html, "2026-2027", courseDescriptionsUrl);

  assert.deepEqual(course.prerequisiteCourseCodes, ["CHEM 111", "ENGL 101", "MATH 121"]);
  assert.deepEqual(course.prerequisiteAlternativeCourseCodeSets, []);
});

test("Rust enrollment parsing expands simple course-code slash shorthand", () => {
  const html = [
    "<h3>MATH& 153 - Calculus III</h3>",
    "<br><br><strong>Enrollment Requirement:</strong> MATH& 151 / 152",
  ].join("");
  const [course] = buildCourseEntries(html, "2026-2027", courseDescriptionsUrl);

  assert.deepEqual(course.prerequisiteCourseCodes, []);
  assert.deepEqual(course.prerequisiteAlternativeCourseCodeSets, [
    ["MATH& 151"],
    ["MATH& 152"],
  ]);
});

test("Rust enrollment parsing stops before the source's Recommended section", () => {
  const html = [
    "<h3>MATH& 153 - Calculus III</h3>",
    "<br><br><strong>Enrollment Requirement:</strong> MATH& 151 Recommended: MATH& 152",
  ].join("");
  const [course] = buildCourseEntries(html, "2026-2027", courseDescriptionsUrl);

  assert.deepEqual(course.prerequisiteCourseCodes, ["MATH& 151"]);
  assert.deepEqual(course.prerequisiteAlternativeCourseCodeSets, []);
});

test("Rust enrollment parsing preserves nested choices, grade context, and concurrent courses", () => {
  const nestedChoicesHtml = [
    "<h3>CHEM 116 - General Chemistry</h3>",
    "<br><br><strong>Enrollment Requirement:</strong> MATH 121 with a grade of 2.0 or higher and (ENGL 101 or CHEM 111)",
  ].join("");
  const concurrentHtml = [
    "<h3>CHEM 117 - General Chemistry</h3>",
    "<br><br><strong>Enrollment Requirement:</strong> MATH 110; concurrent enrollment in MATH 111",
  ].join("");
  const [nestedChoices] = buildCourseEntries(
    nestedChoicesHtml,
    "2026-2027",
    courseDescriptionsUrl
  );
  const [concurrent] = buildCourseEntries(concurrentHtml, "2026-2027", courseDescriptionsUrl);

  assert.deepEqual(nestedChoices.prerequisiteCourseCodes, []);
  assert.deepEqual(nestedChoices.prerequisiteAlternativeCourseCodeSets, [
    ["CHEM 111", "MATH 121"],
    ["ENGL 101", "MATH 121"],
  ]);
  assert.deepEqual(concurrent.prerequisiteCourseCodes, ["MATH 110"]);
  assert.deepEqual(concurrent.corequisiteCourseCodes, ["MATH 111"]);
});

test("course corequisite sections are extracted in Rust and preserved as notes", () => {
  const html = [
    "<h3>CHEM 110 - Preparatory Chemistry</h3>",
    "<br><br><strong>Corequisite:</strong> ENGL 101",
    "<br><br><strong>Course Outcomes:</strong> separate outcome text.",
  ].join("");
  const [course] = buildCourseEntries(html, "2026-2027", courseDescriptionsUrl);

  assert.deepEqual(course.corequisiteNotes, ["Official Green River corequisite text: ENGL 101"]);
  assert.deepEqual(course.corequisiteCourseCodes, ["ENGL 101"]);
  assert.deepEqual(course.corequisiteAlternativeCourseCodeSets, []);
});

test("Rust corequisite parsing preserves simple AND/OR paths and concurrent-enrollment wording", () => {
  const alternativesHtml = [
    "<h3>CHEM 111 - General Chemistry</h3>",
    "<br><br><strong>Corequisite:</strong> MATH 120 and MATH 121 or MATH 122",
  ].join("");
  const fallbackHtml = [
    "<h3>CHEM 112 - General Chemistry II</h3>",
    "<br><br><strong>Corequisite:</strong> MATH 110 or concurrent enrollment in MATH 111",
  ].join("");
  const [alternatives] = buildCourseEntries(
    alternativesHtml,
    "2026-2027",
    courseDescriptionsUrl
  );
  const [fallback] = buildCourseEntries(fallbackHtml, "2026-2027", courseDescriptionsUrl);

  assert.deepEqual(alternatives.corequisiteAlternativeCourseCodeSets, [
    ["MATH 120", "MATH 121"],
    ["MATH 120", "MATH 122"],
  ]);
  assert.deepEqual(fallback.corequisiteCourseCodes, ["MATH 110", "MATH 111"]);
});

test("Rust corequisite parsing expands simple course-code slash shorthand", () => {
  const html = [
    "<h3>CHEM 115 - General Chemistry</h3>",
    "<br><br><strong>Corequisite:</strong> MATH 120/121",
  ].join("");
  const [course] = buildCourseEntries(html, "2026-2027", courseDescriptionsUrl);

  assert.deepEqual(course.corequisiteCourseCodes, []);
  assert.deepEqual(course.corequisiteAlternativeCourseCodeSets, [
    ["MATH 120"],
    ["MATH 121"],
  ]);
  assert.deepEqual(course.corequisiteNotes, [
    "Official Green River corequisite text: MATH 120/121",
  ]);
});

test("course credit and description fields preserve the source text boundaries", () => {
  const html = [
    "<h3>ENGL&amp; 101 - Composition</h3>",
    "Credits: 2.5",
    "<br><p>Writing &amp; research foundations.</p>",
    "<br><br><strong>Course Outcomes:</strong> Outcomes are listed separately.",
  ].join("");
  const [course] = buildCourseEntries(html, "2026-2027", courseDescriptionsUrl);

  assert.equal(course.creditLabel, "2.5");
  assert.equal(course.creditValue, 2.5);
  assert.equal(course.catalogDescription, "Writing & research foundations.");
});

test("legacy general-education classifier export delegates to the Rust parser", () => {
  assert.deepEqual(
    classifyGrcGeneralEducationRequirementTags("English and Diversity"),
    ["AH", "DIV"]
  );
});

test("general-education requirement labels are classified by Rust in catalog tag order", () => {
  const label =
    "Basic Skills / Communication; Quantitative; English; Social Science; Natural Science; Diversity";
  const html = [
    "<h3>ART 101 - Introduction to Art</h3>",
    "<br><br><strong>Satisfies Requirement:</strong>",
    label,
    "<br><br><strong>Course Outcomes:</strong> Outcomes remain outside the label.",
  ].join("");
  const [course] = buildCourseEntries(html, "2026-2027", courseDescriptionsUrl);

  assert.deepEqual(course.grcGeneralEducationCategories, [
    {
      catalogYearLabel: "2026-2027",
      label,
      tags: ["COMM", "QSR", "AH", "SSC", "NSC", "DIV"],
    },
  ]);
  assert.equal(course.catalogDescription, null);
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

test("Rust extracts linked course-code candidates with legacy context filters", () => {
  const html = [
    "<h3>CHEM 115 - General Chemistry</h3>",
    "<br><br><strong>Enrollment Requirement:</strong>",
    '<a href="a">eligible for MATH 121</a>; ',
    '<a href="b">equivalent of MATH 122</a>; ',
    '<a href="c">admission into MATH 123</a>; ',
    '<a href="d">MATH 124 with two years in high school</a>; ',
    '<a href="e">MATH 125/126</a>',
  ].join("");
  const [course] = buildCourseEntries(html, "2026-2027", courseDescriptionsUrl);
  const clickableEvidence = course.prerequisiteNotes.find((note) =>
    note.startsWith("Clickable catalog course-code references")
  );

  assert.equal(
    clickableEvidence,
    "Clickable catalog course-code references within the enrollment requirement: MATH 125, MATH 126."
  );
  assert.doesNotMatch(clickableEvidence, /MATH 12[1-4]/);
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
