const crypto = require("crypto");
const fs = require("fs");
const path = require("path");
const { spawnSync } = require("node:child_process");
const { fetchWithHandling } = require("../lib/fetch-with-handling.cjs");
const { ensureTmpLayout, getTmpPath } = require("../lib/tmp-layout.cjs");

require("ts-node").register({
  skipProject: true,
  transpileOnly: true,
  compilerOptions: {
    module: "CommonJS",
    moduleResolution: "node",
  },
});

let TRANSFER_PLANNER_GENERATED_COURSE_METADATA = [];
try {
  ({ TRANSFER_PLANNER_GENERATED_COURSE_METADATA } = require("../../constants/transfer-planner-source/course-metadata.generated"));
} catch {
  TRANSFER_PLANNER_GENERATED_COURSE_METADATA = [];
}

const REPO_ROOT = path.resolve(__dirname, "..", "..");
const TMP_DIR = ensureTmpLayout(REPO_ROOT).root;
const SNAPSHOT_DIR = getTmpPath(REPO_ROOT, "transfer-planner-equivalency-guide-snapshots");
const OUTPUT_JSON_PATH = getTmpPath(REPO_ROOT, "transfer-planner-equivalency-guide-parse.json");
const OUTPUT_MD_PATH = getTmpPath(REPO_ROOT, "transfer-planner-equivalency-guide-parse.md");
const OUTPUT_TS_PATH = path.resolve(
  REPO_ROOT,
  "constants",
  "transfer-planner-source",
  "equivalency-guide.generated.ts"
);
const OUTPUT_CATALOG_TS_PATH = path.resolve(
  REPO_ROOT,
  "constants",
  "transfer-equivalency-catalog.generated.ts"
);
const EQUIVALENCY_AUTOMATION_MANIFEST_PATH = path.resolve(
  REPO_ROOT,
  "..",
  "automation",
  "equivalency-refresh",
  "Cargo.toml"
);
const GUIDE_URL = "https://admit.washington.edu/apply/transfer/equivalency-guide/green-river/";
const ENTITY_MAP = {
  amp: "&",
  apos: "'",
  nbsp: " ",
  quot: '"',
  "#038": "&",
  "#39": "'",
  "#160": " ",
  "#8211": "-",
  "#8212": "-",
  "#8217": "'",
};
const LEGACY_GRC_CODE_ALIASES = new Map([
  ["MATH& 254", "MATH& 264"],
]);

function applyGrcLegacyCourseCodeAlias(value) {
  return LEGACY_GRC_CODE_ALIASES.get(value) ?? value;
}

function normalizeMetadataCourseCode(code) {
  return applyGrcLegacyCourseCodeAlias(
    normalizeWhitespace(code).toUpperCase().replace(/\s+/g, " ")
  );
}

const GRC_COURSE_TITLE_BY_CODE = new Map(
  TRANSFER_PLANNER_GENERATED_COURSE_METADATA
    .filter((entry) => entry && entry.schoolId === "grc")
    .map((entry) => [normalizeMetadataCourseCode(entry.code), normalizeWhitespace(entry.title)])
    .filter((entry) => entry[0] && entry[1])
);

function decodeHtmlEntities(value) {
  return String(value ?? "").replace(
    /&(?:amp|apos|nbsp|quot|#038|#39|#160|#8211|#8212|#8217);/gi,
    (match) => {
      const key = match.slice(1, -1).toLowerCase();
      return ENTITY_MAP[key] ?? match;
    }
  );
}

function normalizeWhitespace(value) {
  return decodeHtmlEntities(value)
    .replace(/Â§/g, "§")
    .replace(/Â /g, " ")
    .replace(/\u00a0/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function sha256Text(value) {
  return crypto.createHash("sha256").update(value).digest("hex");
}

function parseGuideRows(html) {
  const manifestPath = EQUIVALENCY_AUTOMATION_MANIFEST_PATH;
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
      "parse-equivalency-guide-rows",
    ],
    {
      cwd: REPO_ROOT,
      encoding: "utf8",
      input: JSON.stringify({
        html,
        grcCourseTitles: Object.fromEntries(GRC_COURSE_TITLE_BY_CODE),
      }),
      maxBuffer: 16 * 1024 * 1024,
    }
  );
  if (result.error) {
    throw new Error(`Could not start Rust equivalency-guide parser: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(
      `Rust equivalency-guide parser failed${result.stderr ? `: ${result.stderr.trim()}` : "."}`
    );
  }
  try {
    return JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`Rust equivalency-guide parser returned invalid JSON: ${error.message}`);
  }
}

function buildRule(row) {
  const rule = row.generatedRule;
  if (
    !rule ||
    typeof rule !== "object" ||
    typeof rule.id !== "string" ||
    typeof rule.type !== "string" ||
    !Array.isArray(rule.sourceCourseSets) ||
    !Array.isArray(rule.targetCourseCodes) ||
    !Array.isArray(rule.sourceLinks)
  ) {
    throw new Error(`Rust parser omitted the generated rule for equivalency row ${row.rowNumber}.`);
  }
  return rule;
}

const STRUCTURED_EQUIVALENCY_DERIVATION_SPECS = [
  {
    id: "stem-calculus-current-sequence",
    type: "sequence",
    title: "Current Green River STEM calculus sequence",
    sourceCourseSets: [["MATH& 151", "MATH& 152", "MATH& 163"]],
    acceptanceCategory: "preferred",
    targetOutcome: "UW MATH 124, 125, and 126 transfer path.",
    notes: [
      "This is the current primary calculus path used throughout the planner for STEM transfer planning.",
    ],
    supportRuleSelectors: [
      { kind: "singleCourse", sourceCourseCode: "MATH& 151", targetIncludes: "MATH 124" },
      { kind: "singleCourse", sourceCourseCode: "MATH& 152", targetIncludes: "MATH 125" },
      { kind: "singleCourse", sourceCourseCode: "MATH& 163", targetIncludes: "MATH 126" },
    ],
  },
  {
    id: "stem-calculus-older-sequence",
    type: "alternate-path",
    title: "Older Green River STEM calculus alternative",
    sourceCourseSets: [["MATH& 151", "MATH& 152", "MATH& 153", "MATH& 264"]],
    acceptanceCategory: "legacy-accepted",
    ruleStatus: "legacy",
    weakerThanRuleIds: ["stem-calculus-current-sequence"],
    effectiveYearRanges: [
      {
        startLabel: "legacy-planner-support",
        endLabel: null,
        note: "Retained because current UW equivalency and planner materials still preserve the older calculus route as a valid alternate path.",
      },
    ],
    targetOutcome:
      "UW MATH 124, 125, 126, plus stronger 224 / 2XX treatment when the full older path is completed.",
    plannerWarnings: [
      "Prefer the current MATH& 151 -> MATH& 152 -> MATH& 163 path for new planning unless the student is already on the older MATH& 153 + MATH& 264 route.",
    ],
    notes: [
      "The planner keeps this older path because UW still describes it in some equivalency and legacy advising materials.",
    ],
    supportRuleSelectors: [
      { kind: "singleCourse", sourceCourseCode: "MATH& 151", targetIncludes: "MATH 124" },
      { kind: "singleCourse", sourceCourseCode: "MATH& 152", targetIncludes: "MATH 125" },
      { kind: "exactSourceSet", sourceCourseSet: ["MATH& 153", "MATH& 264"], ruleStatus: "legacy" },
    ],
  },
  {
    id: "general-chemistry-full-sequence",
    type: "full-credit-combo",
    title: "Full general chemistry sequence",
    sourceCourseSets: [["CHEM& 161", "CHEM& 162", "CHEM& 163"]],
    acceptanceCategory: "preferred",
    targetOutcome: "Full strongest general-chemistry transfer outcome used across many STEM majors.",
    notes: [
      "CHEM& 162 plus CHEM& 163 together produce a stronger UW chemistry outcome than isolated single-course treatment.",
    ],
    supportRuleSelectors: [
      { kind: "singleCourse", sourceCourseCode: "CHEM& 161", targetIncludes: "CHEM 142" },
      { kind: "exactSourceSet", sourceCourseSet: ["CHEM& 162", "CHEM& 163"], ruleStatus: "active" },
    ],
  },
  {
    id: "organic-chemistry-full-sequence",
    type: "full-credit-combo",
    title: "Full organic chemistry sequence",
    sourceCourseSets: [["CHEM& 261", "CHEM& 262", "CHEM& 263"]],
    acceptanceCategory: "preferred",
    targetOutcome:
      "Full UW CHEM 237, 238, 239, 241, and 242 package when the full sequence is completed.",
    notes: [
      "The planner keeps the stronger full-sequence rule because partial completion does not preserve the same outcome.",
    ],
    supportRuleSelectors: [
      {
        kind: "exactSourceSet",
        sourceCourseSet: ["CHEM& 261", "CHEM& 262", "CHEM& 263"],
        ruleStatus: "active",
      },
    ],
  },
  {
    id: "biology-majors-full-sequence",
    type: "full-credit-combo",
    title: "Biology majors full sequence",
    sourceCourseSets: [["BIOL& 211", "BIOL& 212", "BIOL& 213"]],
    acceptanceCategory: "preferred",
    targetOutcome: "Full UW BIOL 180, 200, 220, and 2XX package.",
    notes: ["All three courses are required for the strongest biology-major equivalency."],
    supportRuleSelectors: [
      {
        kind: "exactSourceSet",
        sourceCourseSet: ["BIOL& 211", "BIOL& 212", "BIOL& 213"],
        ruleStatus: "active",
      },
    ],
  },
  {
    id: "anatomy-physiology-full-sequence",
    type: "full-credit-combo",
    title: "Anatomy and physiology sequence",
    sourceCourseSets: [["BIOL& 241", "BIOL& 242"]],
    acceptanceCategory: "preferred",
    targetOutcome: "UW BIOL 118, BIOL 119, and NURS 301 equivalency pattern used in health pathways.",
    notes: ["Both courses are needed for the strongest combined outcome."],
    supportRuleSelectors: [
      { kind: "exactSourceSet", sourceCourseSet: ["BIOL& 241", "BIOL& 242"], ruleStatus: "active" },
    ],
  },
  {
    id: "computer-science-new-sequence",
    type: "sequence",
    title: "Current Green River CS sequence",
    sourceCourseSets: [["CS 121", "CS 122", "CS 123"]],
    acceptanceCategory: "preferred",
    targetOutcome: "Primary Green River intro programming sequence used for planning current CS pathways.",
    notes: [
      "The planner treats this as an ordered sequence rather than three unrelated standalone courses.",
    ],
    supportRuleSelectors: [
      { kind: "singleCourse", sourceCourseCode: "CS 121", targetIncludes: "CSE 121" },
      { kind: "singleCourse", sourceCourseCode: "CS 122", targetIncludes: "CSE 122" },
      { kind: "singleCourse", sourceCourseCode: "CS 123", targetIncludes: "CSE 123" },
    ],
  },
  {
    id: "calculus-physics-sequence",
    type: "sequence",
    title: "Calculus-based physics sequence",
    sourceCourseSets: [["PHYS& 221", "PHYS& 222", "PHYS& 223"]],
    acceptanceCategory: "preferred",
    targetOutcome: "Primary calculus-based physics transfer sequence.",
    notes: [
      "The planner keeps this sequence grouped because many engineering majors depend on full completion.",
    ],
    supportRuleSelectors: [
      { kind: "singleCourse", sourceCourseCode: "PHYS& 221", targetIncludes: "PHYS 121" },
      { kind: "singleCourse", sourceCourseCode: "PHYS& 222", targetIncludes: "PHYS 122" },
      { kind: "singleCourse", sourceCourseCode: "PHYS& 223", targetIncludes: "PHYS 123" },
    ],
  },
];

const CHAIN_RULE_DERIVATION_SPECS = [
  {
    id: "chain:MATH-STEM",
    title: "STEM calculus sequence",
    acceptanceCategory: "accepted-with-warning",
    targetOutcome:
      "Broad calculus-sequence reference spanning the current MATH& 151 -> MATH& 152 -> MATH& 163 path and the retained legacy MATH& 153 + MATH& 264 route.",
    plannerWarnings: [
      "Use the explicit structured calculus rules for current-vs-older path decisions. This chain summary is a broad planner reference, not the most precise route selector.",
    ],
    supportRuleSelectors: [
      { kind: "structuredRule", id: "stem-calculus-current-sequence" },
      { kind: "structuredRule", id: "stem-calculus-older-sequence" },
    ],
  },
  {
    id: "chain:CS-LEGACY",
    title: "Legacy CS sequence",
    acceptanceCategory: "legacy-accepted",
    ruleStatus: "legacy",
    weakerThanRuleIds: ["computer-science-new-sequence"],
    effectiveYearRanges: [
      {
        startLabel: "legacy-planner-support",
        endLabel: null,
        note: "Retained because older UW equivalency materials and student histories still reference the CS& 141 -> CS 145 path.",
      },
    ],
    targetOutcome:
      "Legacy Green River CS route centered on CS& 141 and CS 145 for students who already started on the older path.",
    plannerWarnings: [
      "The planner prefers the current CS 121 -> CS 122 -> CS 123 path for new students. Keep the legacy path only when the student already started on it or the published legacy guidance confirms it is the right fit.",
    ],
    supportRuleSelectors: [
      { kind: "singleCourse", sourceCourseCode: "CS& 141", targetIncludes: "CSE 142" },
      { kind: "singleCourse", sourceCourseCode: "CS 145", targetIncludes: "CSE 143" },
    ],
  },
  {
    id: "chain:CHEM-GEN",
    title: "General chemistry sequence",
    acceptanceCategory: "accepted-with-warning",
    targetOutcome: "General chemistry sequence with a stronger combined outcome when the full path is completed.",
    plannerWarnings: [
      "Partial completion yields weaker CHEM 1XX treatment than the stronger full-sequence outcome used by many STEM pathways.",
    ],
    supportRuleSelectors: [{ kind: "structuredRule", id: "general-chemistry-full-sequence" }],
  },
  {
    id: "chain:CHEM-ORG",
    title: "Organic chemistry sequence",
    acceptanceCategory: "accepted-with-warning",
    targetOutcome: "Organic chemistry sequence with the strongest UW outcome reserved for the full three-course path.",
    plannerWarnings: [
      "The strongest UW organic chemistry outcome depends on the full CHEM& 261 + 262 + 263 sequence rather than isolated single-course treatment.",
    ],
    supportRuleSelectors: [{ kind: "structuredRule", id: "organic-chemistry-full-sequence" }],
  },
  {
    id: "chain:BIO-MAJORS",
    title: "Biology majors sequence",
    acceptanceCategory: "accepted-with-warning",
    targetOutcome: "Biology majors sequence with the strongest outcome reserved for all three majors-biology courses together.",
    plannerWarnings: [
      "The strongest biology-major transfer outcome depends on completing BIOL& 211 + 212 + 213 as a full sequence.",
    ],
    supportRuleSelectors: [{ kind: "structuredRule", id: "biology-majors-full-sequence" }],
  },
  {
    id: "chain:BIO-ANAT",
    title: "Anatomy and physiology sequence",
    acceptanceCategory: "accepted-with-warning",
    targetOutcome:
      "Anatomy and physiology sequence with the strongest combined UW outcome reserved for both courses together.",
    plannerWarnings: [
      "The combined UW anatomy and physiology outcome depends on completing both BIOL& 241 and BIOL& 242.",
    ],
    supportRuleSelectors: [{ kind: "structuredRule", id: "anatomy-physiology-full-sequence" }],
  },
  {
    id: "chain:ACCT-COMBO",
    title: "Accounting full-credit combo",
    acceptanceCategory: "accepted-with-warning",
    targetOutcome: "Combined accounting sequence rule for the stronger ACCTG 215 transfer outcome.",
    plannerWarnings: [
      "The stronger UW accounting outcome depends on ACCT& 201 + ACCT& 202 together rather than isolated single-course treatment.",
    ],
    supportRuleSelectors: [
      { kind: "exactSourceSet", sourceCourseSet: ["ACCT& 201", "ACCT& 202"], ruleStatus: "active" },
    ],
  },
  {
    id: "chain:ASTR-COMBO",
    title: "Astronomy full-credit combo",
    acceptanceCategory: "accepted-with-warning",
    targetOutcome: "Combined astronomy rule where the second course changes the final UW credit mix.",
    plannerWarnings: [
      "The second astronomy course changes the final UW credit outcome, so treat this as a conditional combo instead of two interchangeable standalone classes.",
    ],
    supportRuleSelectors: [
      { kind: "exactSourceSet", sourceCourseSet: ["ASTR& 100", "ASTR& 101"], ruleStatus: "active" },
    ],
  },
  {
    id: "chain:HIST-US",
    title: "US history full-credit combo",
    acceptanceCategory: "accepted-with-warning",
    targetOutcome: "US history combo rule for the stronger combined HSTAA outcome.",
    plannerWarnings: [
      "The full UW US-history outcome depends on HIST& 136 + HIST& 137 together rather than one course alone.",
    ],
    supportRuleSelectors: [
      { kind: "exactSourceSet", sourceCourseSet: ["HIST& 136", "HIST& 137"], ruleStatus: "active" },
    ],
  },
  {
    id: "chain:ENGL-250",
    title: "English 250 full-credit combo",
    acceptanceCategory: "accepted-with-warning",
    targetOutcome: "English combo rule for the stronger ENGL 250 outcome.",
    plannerWarnings: [
      "The stronger ENGL 250 outcome depends on ENGL& 244 + ENGL& 245 together.",
    ],
    supportRuleSelectors: [
      { kind: "exactSourceSet", sourceCourseSet: ["ENGL& 244", "ENGL& 245"], ruleStatus: "active" },
    ],
  },
  {
    id: "chain:COMM-266",
    title: "CMST 266 credit rule",
    acceptanceCategory: "accepted-with-warning",
    targetOutcome: "Conditional CMST 266 credit rule where the full 5-credit version yields CMS 272.",
    plannerWarnings: [
      "CMST 266 only yields CMS 272 when it is taken for 5 credits. Otherwise it remains CMS 2XX credit.",
    ],
    supportRuleSelectors: [
      { kind: "exactSourceSet", sourceCourseSet: ["CMST 266"], ruleStatus: "active" },
    ],
  },
  {
    id: "chain:NATRS-COMBO",
    title: "Natural resources ESRM combo",
    acceptanceCategory: "accepted-with-warning",
    targetOutcome:
      "Natural resources combo rule where NATRS 180 + NATRS 292 produces a special ESRM-major outcome.",
    plannerWarnings: [
      "NATRS 180 + NATRS 292 has a special combined ESRM-major rule, so do not treat the two courses as interchangeable standalone credits.",
    ],
    supportRuleSelectors: [
      { kind: "exactSourceSet", sourceCourseSet: ["NATRS 180", "NATRS 292"], ruleStatus: "active" },
    ],
  },
];

function synthesizeDerivedRulesInRust({ officialRules, structuredRules = [], specs }) {
  const manifestPath = EQUIVALENCY_AUTOMATION_MANIFEST_PATH;
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
      "synthesize-uw-grc-derived-rules",
    ],
    {
      cwd: REPO_ROOT,
      encoding: "utf8",
      input: JSON.stringify({
        officialRules,
        structuredRules,
        items: specs.map(({ supportRuleSelectors, ...spec }) => ({ spec, supportRuleSelectors })),
      }),
      maxBuffer: 16 * 1024 * 1024,
    }
  );
  if (result.error) {
    throw new Error(`Could not start Rust equivalency-rule synthesizer: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(
      `Rust equivalency-rule synthesizer failed${result.stderr ? `: ${result.stderr.trim()}` : "."}`
    );
  }
  try {
    const rules = JSON.parse(result.stdout);
    if (!Array.isArray(rules)) {
      throw new Error("expected an array of generated rules");
    }
    return rules;
  } catch (error) {
    throw new Error(`Rust equivalency-rule synthesizer returned invalid JSON: ${error.message}`);
  }
}

function buildDerivedStructuredEquivalencyRules(officialRules) {
  return synthesizeDerivedRulesInRust({
    officialRules,
    specs: STRUCTURED_EQUIVALENCY_DERIVATION_SPECS,
  }).sort((left, right) => left.id.localeCompare(right.id));
}

function buildDerivedChainRules(officialRules, structuredRules) {
  return synthesizeDerivedRulesInRust({
    officialRules,
    structuredRules,
    specs: CHAIN_RULE_DERIVATION_SPECS,
  }).sort((left, right) => left.id.localeCompare(right.id));
}

function buildEquivalencyGuideReportInRust(input) {
  const manifestPath = EQUIVALENCY_AUTOMATION_MANIFEST_PATH;
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
      "build-equivalency-guide-report",
    ],
    {
      cwd: REPO_ROOT,
      encoding: "utf8",
      input: JSON.stringify(input),
      maxBuffer: 16 * 1024 * 1024,
    }
  );
  if (result.error) {
    throw new Error(`Could not start Rust equivalency-guide report builder: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(
      `Rust equivalency-guide report builder failed${result.stderr ? `: ${result.stderr.trim()}` : "."}`
    );
  }
  try {
    const output = JSON.parse(result.stdout);
    if (!output?.report || typeof output.markdown !== "string") {
      throw new Error("expected report and Markdown output");
    }
    return output;
  } catch (error) {
    throw new Error(`Rust equivalency-guide report builder returned invalid JSON: ${error.message}`);
  }
}

function renderGeneratedRules(scope) {
  const manifestPath = EQUIVALENCY_AUTOMATION_MANIFEST_PATH;
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
      "render-equivalency-guide-typescript",
    ],
    {
      cwd: REPO_ROOT,
      encoding: "utf8",
      input: JSON.stringify(scope),
      maxBuffer: 16 * 1024 * 1024,
    }
  );
  if (result.error) {
    throw new Error(`Could not start Rust equivalency TypeScript renderer: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(
      `Rust equivalency TypeScript renderer failed${result.stderr ? `: ${result.stderr.trim()}` : "."}`
    );
  }
  return result.stdout;
}

function renderGeneratedCatalog(scope) {
  const manifestPath = EQUIVALENCY_AUTOMATION_MANIFEST_PATH;
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
      "render-transfer-equivalency-catalog-typescript",
    ],
    {
      cwd: REPO_ROOT,
      encoding: "utf8",
      input: JSON.stringify(scope),
      maxBuffer: 16 * 1024 * 1024,
    }
  );
  if (result.error) {
    throw new Error(`Could not start Rust transfer-equivalency catalog renderer: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(
      `Rust transfer-equivalency catalog renderer failed${result.stderr ? `: ${result.stderr.trim()}` : "."}`
    );
  }
  return result.stdout;
}

function writeGeneratedRules(scope) {
  const generatedSource = renderGeneratedRules(scope);
  const generatedCatalog = renderGeneratedCatalog(scope);
  fs.mkdirSync(path.dirname(OUTPUT_TS_PATH), { recursive: true });
  fs.writeFileSync(OUTPUT_TS_PATH, generatedSource);
  fs.writeFileSync(OUTPUT_CATALOG_TS_PATH, generatedCatalog);
}

function writeReports({ report, markdown }) {
  fs.mkdirSync(TMP_DIR, { recursive: true });
  fs.writeFileSync(OUTPUT_JSON_PATH, `${JSON.stringify(report, null, 2)}\n`);
  fs.writeFileSync(OUTPUT_MD_PATH, markdown);
}

async function fetchGuideHtml() {
  const response = await fetchWithHandling(GUIDE_URL, {
    operation: "Fetch UW Green River equivalency guide",
    throwOnHttpError: false,
    timeoutMs: 30000,
    userAgent: "GatorGuideTransferPlannerEquivalencyIngest/1.0",
  });
  const html = await response.text();
  if (!response.ok) {
    throw new Error(`Failed to fetch ${GUIDE_URL}: ${response.status} ${response.statusText}`);
  }

  fs.mkdirSync(SNAPSHOT_DIR, { recursive: true });
  const snapshotPath = path.resolve(SNAPSHOT_DIR, "uw-green-river-equivalency-guide.html");
  fs.writeFileSync(snapshotPath, html);

  return {
    html,
    status: response.status,
    finalUrl: response.url,
    contentType: response.headers.get("content-type"),
    snapshotPath,
  };
}

async function main() {
  const fetched = await fetchGuideHtml();
  const rows = parseGuideRows(fetched.html);
  const officialRules = rows.map(buildRule).sort((left, right) => left.id.localeCompare(right.id));
  const structuredDerivedRules = buildDerivedStructuredEquivalencyRules(officialRules);
  const chainDerivedRules = buildDerivedChainRules(officialRules, structuredDerivedRules);
  const derivedRules = [...structuredDerivedRules, ...chainDerivedRules].sort((left, right) =>
    left.id.localeCompare(right.id)
  );
  const { report, markdown } = buildEquivalencyGuideReportInRust({
    generatedAt: new Date().toISOString(),
    sourceUrl: GUIDE_URL,
    status: fetched.status,
    finalUrl: fetched.finalUrl,
    contentType: fetched.contentType,
    snapshotPath: path.relative(REPO_ROOT, fetched.snapshotPath).replace(/\\/g, "/"),
    snapshotSha256: sha256Text(fetched.html),
    rows,
    officialRules,
    derivedRules,
  });

  writeGeneratedRules({ officialRules, derivedRules });
  writeReports({ report, markdown });

  console.log(
    JSON.stringify(
      {
        officialRules: report.officialRuleCount,
        derivedRules: report.derivedRuleCount,
        generatedRules: report.ruleCount,
        departments: report.departmentCount,
        directCourseRules: report.countsByType["direct-course"] ?? 0,
        sequenceRules: report.countsByType.sequence ?? 0,
        chainRules: report.countsByType["chain-rule"] ?? 0,
        noCreditRules: report.countsByType["no-credit"] ?? 0,
        limitedCreditRules: report.countsByType["limited-credit"] ?? 0,
        output: path.relative(REPO_ROOT, OUTPUT_TS_PATH).replace(/\\/g, "/"),
        catalogOutput: path.relative(REPO_ROOT, OUTPUT_CATALOG_TS_PATH).replace(/\\/g, "/"),
        report: path.relative(REPO_ROOT, OUTPUT_JSON_PATH).replace(/\\/g, "/"),
      },
      null,
      2
    )
  );
}

if (require.main === module) {
  main().catch((error) => {
    console.error(error);
    process.exit(1);
  });
}

module.exports = {
  buildDerivedStructuredEquivalencyRules,
  buildDerivedChainRules,
};
