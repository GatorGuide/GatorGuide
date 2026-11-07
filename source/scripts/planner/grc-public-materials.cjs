const fs = require("fs");
const path = require("path");
const { spawnSync } = require("child_process");
const { fetchTextWithHandling } = require("../lib/fetch-with-handling.cjs");
const {
  SOURCE_ROOT,
  ensurePlannerTmpLayout,
  getPlannerTmpPath,
  hasArg,
  writePlannerJsonReport,
  writePlannerMarkdownReport,
} = require("./lib/script-harness.cjs");

const REPO_ROOT = SOURCE_ROOT;
ensurePlannerTmpLayout();
const OUTPUT_JSON_PATH = getPlannerTmpPath("transfer-planner-grc-public-materials.json");
const OUTPUT_MD_PATH = getPlannerTmpPath("transfer-planner-grc-public-materials.md");

const GRC_CLASS_SCHEDULES_URL =
  "https://www.greenriver.edu/students/academics/class-schedules-catalog/index.html";
const GRC_CATALOG_ARCHIVE_URL =
  "https://www.greenriver.edu/students/academics/class-schedules-catalog/catalog-archive.html";
const GRC_CATALOG_ROOT_URL = "https://catalog.greenriver.edu/";
const USER_AGENT = "GatorGuideTransferPlannerGrcDiscovery/1.0";

function ensureTmpDir() {
  ensurePlannerTmpLayout();
}

function compareAcademicYearLabels(left, right) {
  const leftStart = getAcademicYearStart(left);
  const rightStart = getAcademicYearStart(right);

  if (Number.isFinite(leftStart) && Number.isFinite(rightStart) && leftStart !== rightStart) {
    return leftStart - rightStart;
  }

  return String(left ?? "").localeCompare(String(right ?? ""));
}

function getAcademicYearStart(label) {
  const match = String(label ?? "").match(/^(20\d{2})-/);
  return match ? Number.parseInt(match[1], 10) : Number.NaN;
}

function sortAcademicYearLabels(labels, direction = "asc") {
  const multiplier = direction === "desc" ? -1 : 1;
  return [...labels].sort((left, right) => multiplier * compareAcademicYearLabels(left, right));
}

function filterRelevantAnnualSchedules(annualSchedules, currentCatalogYearLabel) {
  const schedules = runRustGrcParser(
    "filter-grc-annual-schedules",
    { annualSchedules, currentCatalogYearLabel },
    "GRC annual-schedule filter"
  );
  if (!Array.isArray(schedules)) {
    throw new Error("Rust GRC annual-schedule filter returned invalid schedule data");
  }
  return schedules;
}

function slugifyAcademicYear(label) {
  return String(label ?? "")
    .trim()
    .replace(/[^0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

function buildAnnualScheduleSnapshotPath(label) {
  return getPlannerTmpPath(`${label}-Annual-Schedule.pdf`);
}

async function fetchText(url) {
  return fetchTextWithHandling(url, {
    operation: "GRC public materials fetch",
    timeoutMs: 30000,
    userAgent: USER_AGENT,
  });
}

function runRustGrcParser(command, input, parserLabel) {
  const manifestPath = path.resolve(REPO_ROOT, "..", "automation", "Cargo.toml");
  const result = spawnSync(
    "cargo",
    [
      "run",
      "--offline",
      "--locked",
      "--quiet",
      "--manifest-path",
      manifestPath,
      "--",
      command,
    ],
    {
      cwd: REPO_ROOT,
      encoding: "utf8",
      input: JSON.stringify(input),
      maxBuffer: 1024 * 1024,
    }
  );
  if (result.error) {
    throw new Error(`Could not start Rust ${parserLabel} parser: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(
      `Rust ${parserLabel} parser failed${result.stderr ? `: ${result.stderr.trim()}` : "."}`
    );
  }
  try {
    return JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`Rust ${parserLabel} parser returned invalid JSON: ${error.message}`);
  }
}

function extractGrcAnnualSchedules(html, baseUrl = GRC_CLASS_SCHEDULES_URL) {
  const candidates = runRustGrcParser(
    "parse-grc-annual-schedule-candidates",
    { html, baseUrl },
    "GRC annual-schedule"
  );
  if (
    !Array.isArray(candidates) ||
    candidates.some(
      (candidate) =>
        !candidate ||
        typeof candidate.label !== "string" ||
        typeof candidate.url !== "string" ||
        typeof candidate.sourcePageUrl !== "string"
    )
  ) {
    throw new Error("Rust GRC annual-schedule parser returned invalid candidate data");
  }
  return candidates.map(({ label, url, sourcePageUrl }) => ({
    label,
    url,
    outputPath: buildAnnualScheduleSnapshotPath(label),
    ownerId: `grc-annual-schedule-${slugifyAcademicYear(label)}`,
    sourcePageUrl,
  }));
}

function extractGrcCatalogArchiveEntries(html, baseUrl = GRC_CATALOG_ARCHIVE_URL) {
  const entries = runRustGrcParser(
    "parse-grc-catalog-archive-entries",
    { html, baseUrl },
    "GRC catalog-archive"
  );
  if (
    !Array.isArray(entries) ||
    entries.some(
      (entry) =>
        !entry ||
        typeof entry.label !== "string" ||
        typeof entry.url !== "string" ||
        typeof entry.sourcePageUrl !== "string"
    )
  ) {
    throw new Error("Rust GRC catalog-archive parser returned invalid entry data");
  }
  return entries;
}

function extractCurrentGrcCatalogDetails(
  html,
  catalogRootUrl,
  expectedCatalogYearLabel
) {
  const currentCatalog = runRustGrcParser(
    "parse-grc-current-catalog-details",
    { html, catalogRootUrl, expectedCatalogYearLabel },
    "GRC current-catalog"
  );
  const requiredStringFields = [
    "label",
    "rootUrl",
    "sourcePageUrl",
    "courseDescriptionsUrl",
    "courseDescriptionsExpandedUrl",
  ];
  if (
    !currentCatalog ||
    typeof currentCatalog !== "object" ||
    requiredStringFields.some((field) => typeof currentCatalog[field] !== "string") ||
    (currentCatalog.catoid !== null && typeof currentCatalog.catoid !== "string") ||
    (currentCatalog.navoid !== null && typeof currentCatalog.navoid !== "string")
  ) {
    throw new Error("Rust GRC current-catalog parser returned invalid catalog details");
  }
  return currentCatalog;
}

function buildPagedGrcCourseDescriptionsUrl(courseDescriptionsUrl, pageNumber) {
  const url = new URL(courseDescriptionsUrl);
  url.searchParams.set("filter[item_type]", "3");
  url.searchParams.set("filter[only_active]", "1");
  url.searchParams.set("filter[3]", "1");
  if (pageNumber > 1) {
    url.searchParams.set("filter[cpage]", String(pageNumber));
  } else {
    url.searchParams.delete("filter[cpage]");
  }
  url.searchParams.set("expand", "1");
  url.searchParams.set("print", "");
  return url.href;
}

function loadCachedGrcPublicMaterials() {
  if (!fs.existsSync(OUTPUT_JSON_PATH)) {
    return null;
  }

  const cached = JSON.parse(fs.readFileSync(OUTPUT_JSON_PATH, "utf8"));
  return {
    ...cached,
    annualSchedules: Array.isArray(cached.annualSchedules)
      ? cached.annualSchedules.map((entry) => ({
          ...entry,
          outputPath: buildAnnualScheduleSnapshotPath(entry.label),
        }))
      : [],
  };
}

function writeMarkdown(materials) {
  const lines = [
    "# Green River Public Materials Discovery",
    "",
    `Generated: ${materials.generatedAt}`,
    `Used cached fallback: ${materials.usedSnapshotFallback ? "yes" : "no"}`,
    ...(materials.snapshotFallbackReason
      ? [`Fallback reason: ${materials.snapshotFallbackReason}`]
      : []),
    "",
    `- Class schedules page: ${materials.discoveryPages.classSchedulesUrl}`,
    `- Catalog archive page: ${materials.discoveryPages.catalogArchiveUrl}`,
    `- Current catalog root: ${materials.currentCatalog.rootUrl}`,
    `- Current catalog year: ${materials.currentCatalog.label}`,
    `- Current course descriptions: ${materials.currentCatalog.courseDescriptionsUrl}`,
    "",
    "## Annual Schedules",
    "",
    ...materials.annualSchedules.map(
      (entry) => `- ${entry.label}: ${entry.url}`
    ),
    "",
    "## Catalog Archive Entries",
    "",
    ...materials.catalogEntries.map((entry) => `- ${entry.label}: ${entry.url}`),
    "",
  ];

  writePlannerMarkdownReport(OUTPUT_MD_PATH, lines);
}

async function discoverLiveGrcPublicMaterials(fetchPageText = fetchText) {
  const classSchedulesHtml = await fetchPageText(GRC_CLASS_SCHEDULES_URL);
  const discoveredAnnualSchedules = extractGrcAnnualSchedules(
    classSchedulesHtml,
    GRC_CLASS_SCHEDULES_URL
  );
  if (!discoveredAnnualSchedules.length) {
    throw new Error(`No Green River annual schedule PDFs were found on ${GRC_CLASS_SCHEDULES_URL}`);
  }

  const catalogArchiveHtml = await fetchPageText(GRC_CATALOG_ARCHIVE_URL);
  const catalogEntries = extractGrcCatalogArchiveEntries(catalogArchiveHtml, GRC_CATALOG_ARCHIVE_URL);
  if (!catalogEntries.length) {
    throw new Error(`No Green River catalog archive entries were found on ${GRC_CATALOG_ARCHIVE_URL}`);
  }

  const currentCatalogEntry = catalogEntries[0];
  const currentCatalogRootHtml = await fetchPageText(currentCatalogEntry.url);
  const currentCatalog = extractCurrentGrcCatalogDetails(
    currentCatalogRootHtml,
    currentCatalogEntry.url,
    currentCatalogEntry.label
  );
  const annualSchedules = filterRelevantAnnualSchedules(
    discoveredAnnualSchedules,
    currentCatalog.label
  );

  return {
    generatedAt: new Date().toISOString(),
    usedSnapshotFallback: false,
    snapshotFallbackReason: null,
    discoveryPages: {
      classSchedulesUrl: GRC_CLASS_SCHEDULES_URL,
      catalogArchiveUrl: GRC_CATALOG_ARCHIVE_URL,
    },
    annualSchedules,
    discoveredAnnualScheduleCount: discoveredAnnualSchedules.length,
    catalogEntries,
    currentCatalog,
  };
}

async function loadGrcPublicMaterials(options = {}) {
  const {
    forceRefresh = false,
    allowSnapshotFallback = true,
    cacheOnly = process.env.GATORGUIDE_PLANNER_CACHE_ONLY === "1",
    fetchPageText = fetchText,
  } = options;
  ensureTmpDir();

  const cached = loadCachedGrcPublicMaterials();
  if (cached && (!forceRefresh || cacheOnly)) {
    return cached;
  }

  if (cacheOnly) {
    throw new Error(
      `Cached Green River public-materials discovery is required in no-download mode, but ${OUTPUT_JSON_PATH} was not found. Run the normal planner refresh once to create it.`
    );
  }

  try {
    const materials = await discoverLiveGrcPublicMaterials(fetchPageText);
    writePlannerJsonReport(OUTPUT_JSON_PATH, materials);
    writeMarkdown(materials);
    return materials;
  } catch (error) {
    if (!allowSnapshotFallback) {
      throw error;
    }

    const cached = loadCachedGrcPublicMaterials();
    if (!cached) {
      throw error;
    }

    const fallback = {
      ...cached,
      generatedAt: new Date().toISOString(),
      usedSnapshotFallback: true,
      snapshotFallbackReason: error.message,
    };
    writePlannerJsonReport(OUTPUT_JSON_PATH, fallback);
    writeMarkdown(fallback);
    return fallback;
  }
}

async function main() {
  const materials = await loadGrcPublicMaterials({
    forceRefresh: hasArg("--refresh"),
    allowSnapshotFallback: !hasArg("--no-snapshot-fallback"),
  });

  console.log(
    `Discovered ${materials.annualSchedules.length} Green River annual schedules and ${materials.catalogEntries.length} catalog archive entries.`
  );
  console.log(`Current catalog year: ${materials.currentCatalog.label}`);
  console.log(`Discovery JSON: ${OUTPUT_JSON_PATH}`);
  console.log(`Discovery Markdown: ${OUTPUT_MD_PATH}`);
}

if (require.main === module) {
  main().catch((error) => {
    console.error(error);
    process.exit(1);
  });
}

module.exports = {
  GRC_CLASS_SCHEDULES_URL,
  GRC_CATALOG_ARCHIVE_URL,
  GRC_CATALOG_ROOT_URL,
  OUTPUT_JSON_PATH,
  OUTPUT_MD_PATH,
  buildAnnualScheduleSnapshotPath,
  buildPagedGrcCourseDescriptionsUrl,
  compareAcademicYearLabels,
  filterRelevantAnnualSchedules,
  extractCurrentGrcCatalogDetails,
  extractGrcAnnualSchedules,
  extractGrcCatalogArchiveEntries,
  getAcademicYearStart,
  loadGrcPublicMaterials,
  sortAcademicYearLabels,
};
