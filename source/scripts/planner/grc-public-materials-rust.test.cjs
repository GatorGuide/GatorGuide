const assert = require("node:assert/strict");
const test = require("node:test");
const {
  GRC_CLASS_SCHEDULES_URL,
  GRC_CATALOG_ARCHIVE_URL,
  buildAnnualScheduleSnapshotPath,
  extractCurrentGrcCatalogDetails,
  extractGrcAnnualSchedules,
  extractGrcCatalogArchiveEntries,
  filterRelevantAnnualSchedules,
} = require("./grc-public-materials.cjs");

test("current GRC catalog detail parsing delegates to Rust and preserves the URL contract", () => {
  const catalogRootUrl = "https://catalog.greenriver.edu/";
  const currentCatalog = extractCurrentGrcCatalogDetails(
    '<a href="/content.php?catoid=10&amp;navoid=624">Course Descriptions</a>',
    catalogRootUrl,
    "2025-2026"
  );

  assert.deepEqual(currentCatalog, {
    label: "2025-2026",
    rootUrl: catalogRootUrl,
    sourcePageUrl: catalogRootUrl,
    courseDescriptionsUrl:
      "https://catalog.greenriver.edu/content.php?catoid=10&navoid=624",
    courseDescriptionsExpandedUrl:
      "https://catalog.greenriver.edu/content.php?catoid=10&navoid=624&expand=1&print=",
    catoid: "10",
    navoid: "624",
  });
});

test("GRC annual schedule links are parsed by Rust with legacy metadata and ordering", () => {
  const schedules = extractGrcAnnualSchedules(
    [
      '<a href="/students/media/documents/schedules-and-catalog/2026-2027%20Annual%20Schedule.pdf">2026-2027 Annual Schedule (PDF)</a>',
      '<a href="/students/media/documents/schedules-and-catalog/2025-2026%20Annual%20Schedule%20w%20Cover.pdf">2025-2026 Annual Schedule (PDF)</a>',
      '<a href="https://example.com/2027-2028%20Annual%20Schedule.pdf">2027-2028 Annual Schedule</a>',
      '<a href="/2028-2029-catalog.pdf">2028-2029 Catalog</a>',
    ].join(""),
    GRC_CLASS_SCHEDULES_URL
  );

  assert.deepEqual(schedules, [
    {
      label: "2025-2026",
      url: "https://www.greenriver.edu/students/media/documents/schedules-and-catalog/2025-2026%20Annual%20Schedule%20w%20Cover.pdf",
      outputPath: buildAnnualScheduleSnapshotPath("2025-2026"),
      ownerId: "grc-annual-schedule-2025-2026",
      sourcePageUrl: GRC_CLASS_SCHEDULES_URL,
    },
    {
      label: "2026-2027",
      url: "https://www.greenriver.edu/students/media/documents/schedules-and-catalog/2026-2027%20Annual%20Schedule.pdf",
      outputPath: buildAnnualScheduleSnapshotPath("2026-2027"),
      ownerId: "grc-annual-schedule-2026-2027",
      sourcePageUrl: GRC_CLASS_SCHEDULES_URL,
    },
  ]);
});

test("GRC relevant annual-schedule filtering delegates to Rust and preserves opaque fields", () => {
  const schedules = [
    { label: "2020-2021", payload: "older" },
    { label: "2024-2025", payload: "recent prior" },
    { label: "2025-2026", payload: "current" },
    { label: "2026-2027", payload: "upcoming" },
  ];

  assert.deepEqual(
    filterRelevantAnnualSchedules(schedules, "2025-2026"),
    schedules.slice(1)
  );
  assert.deepEqual(
    filterRelevantAnnualSchedules(schedules, "unknown catalog year"),
    schedules
  );
});

test("GRC catalog archive entries are parsed by Rust with legacy ordering and shape", () => {
  const entries = extractGrcCatalogArchiveEntries(
    [
      '<a href="/catalog-2024">Green River College 2024-2025 Catalog</a>',
      '<a href="https://catalog.greenriver.edu/catalog-2025">Green River College 2025-2026 Catalog</a>',
    ].join(""),
    GRC_CATALOG_ARCHIVE_URL
  );

  assert.deepEqual(entries, [
    {
      label: "2025-2026",
      url: "https://catalog.greenriver.edu/catalog-2025",
      sourcePageUrl: GRC_CATALOG_ARCHIVE_URL,
    },
    {
      label: "2024-2025",
      url: "https://www.greenriver.edu/catalog-2024",
      sourcePageUrl: GRC_CATALOG_ARCHIVE_URL,
    },
  ]);
});
