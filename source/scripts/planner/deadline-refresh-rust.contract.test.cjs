const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const test = require("node:test");

const sourceRoot = path.resolve(__dirname, "..", "..");
const packageJson = JSON.parse(
  fs.readFileSync(path.join(sourceRoot, "package.json"), "utf8")
);
const pipeline = fs.readFileSync(
  path.join(sourceRoot, "scripts/planner/refresh-transfer-planner-sources.cjs"),
  "utf8"
);

test("deadline-page parsing and catalog updates are delegated to Rust", () => {
  assert.match(
    packageJson.scripts["planner:refresh-deadlines"],
    /^cargo run --offline --locked --manifest-path \.\.\/automation\/Cargo\.toml -- refresh-deadlines$/
  );
  assert.match(
    pipeline,
    /runCommand\("cargo",\s*\[\s*"run",\s*"--offline",\s*"--locked",\s*"--manifest-path",\s*"\.\.\/automation\/Cargo\.toml",\s*"--",\s*"refresh-deadlines"/
  );
  assert.equal(
    fs.existsSync(path.join(__dirname, "refresh-deadline-sources.cjs")),
    false,
    "the superseded Node deadline parser must not remain as an alternate implementation"
  );
  assert.equal(
    fs.existsSync(path.resolve(sourceRoot, "../automation/src/planner/deadlines.rs")),
    true
  );
});

test("UW official catalog ingestion is implemented in Rust with a registry-only Node bridge", () => {
  assert.match(
    packageJson.scripts["planner:ingest-uw-catalog"],
    /^cargo run --offline --locked --manifest-path \.\.\/automation\/Cargo\.toml -- ingest-uw-catalog$/
  );
  assert.match(
    pipeline,
    /runCommand\("cargo",\s*\[\s*"run",\s*"--offline",\s*"--locked",\s*"--manifest-path",\s*"\.\.\/automation\/Cargo\.toml",\s*"--",\s*"ingest-uw-catalog"/
  );
  const legacyEntryPoint = fs.readFileSync(
    path.join(__dirname, "ingest-uw-catalog.cjs"),
    "utf8"
  );
  assert.match(legacyEntryPoint, /spawnSync\("cargo"/);
  assert.doesNotMatch(legacyEntryPoint, /fetchTextWithHandling|parseSubjectPage/);
  assert.equal(
    fs.existsSync(path.join(__dirname, "export-transfer-planner-uw-course-codes.cjs")),
    true
  );
  assert.equal(
    fs.existsSync(path.resolve(sourceRoot, "../automation/src/planner/uw_catalog.rs")),
    true
  );
});

test("Green River catalog pagination parsing is delegated to Rust", () => {
  const parser = fs.readFileSync(path.join(__dirname, "ingest-grc-catalog.cjs"), "utf8");
  const pageParser = parser.match(/function detectCatalogPageNumbers\(html\) \{([\s\S]*?)\n\}/)?.[1] ?? "";
  assert.match(pageParser, /parse-grc-catalog-page-numbers/);
  assert.doesNotMatch(pageParser, /matchAll|filter\(Number\.isFinite\)/);
  assert.equal(
    fs.existsSync(path.resolve(sourceRoot, "../automation/src/planner/grc_public_materials.rs")),
    true
  );
});

test("Green River course headings, catalog fields, and labeled sections are parsed in Rust", () => {
  const parser = fs.readFileSync(path.join(__dirname, "ingest-grc-catalog.cjs"), "utf8");
  const detailsParser = parser.match(/function parseGrcCourseDetails\(rawBodies\) \{([\s\S]*?)\n\}/)?.[1] ?? "";
  assert.match(detailsParser, /parse-grc-course-details/);
  const catalogParser = parser.match(/function parseGrcCourseCatalog\(html\) \{([\s\S]*?)\n\}/)?.[1] ?? "";
  assert.match(catalogParser, /parse-grc-course-catalog/);
  assert.match(parser, /function buildCourseEntries\(html, catalogYearLabel, courseDescriptionsSourceUrl\) \{\s*const courseSections = parseGrcCourseCatalog\(html\)/);
  assert.match(parser, /courseSection\.courseCodeCandidate/);
  assert.match(parser, /courseSection\.courseTitleCandidate/);
  assert.match(parser, /courseSection\.courseCodeSyntaxValid/);
  assert.doesNotMatch(parser, /headingText\.match|test\(rawCode\)/);
  assert.match(parser, /courseDetailsForHeading\.catalogDescription/);
  assert.match(parser, /courseDetailsForHeading\.enrollmentRequirementLinkedCourseCodeCandidates/);
  assert.match(parser, /courseDetailsForHeading\.enrollmentRequirement/);
  assert.match(parser, /courseDetailsForHeading\.enrollmentCourseCodePaths/);
  assert.match(parser, /parseGrcEnrollmentRequirementText\(enrollmentRequirement\)/);
  assert.match(parser, /materializeParsedRequirementFields\(uniqueCoursePaths\(rustEnrollmentPaths\), \[\]\)/);
  assert.match(parser, /courseDetailsForHeading\.corequisiteRequirement/);
  assert.match(parser, /courseDetailsForHeading\.corequisiteCourseCodePaths/);
  assert.match(parser, /parseGrcCorequisiteText\(corequisiteRequirement\)/);
  assert.match(parser, /materializeParsedRequirementFields\(\[\], uniqueCoursePaths\(rustCorequisitePaths\)\)/);
  assert.match(parser, /courseDetailsForHeading\.satisfiesRequirement/);
  assert.match(parser, /courseDetailsForHeading\.generalEducationTags/);
  const classifier = parser.match(
    /function classifyGrcGeneralEducationRequirementTags\(value\) \{([\s\S]*?)\n\}/
  )?.[1] ?? "";
  assert.match(classifier, /parseGrcCourseDetails/);
  const rustParserPath = path.resolve(sourceRoot, "../automation/src/planner/grc_catalog.rs");
  const rustParser = fs.readFileSync(rustParserPath, "utf8");
  assert.match(rustParser, /fn extract_linked_course_code_candidates\(/);
  assert.match(rustParser, /ignored_prefix_pattern\.is_match\(before\)/);
  assert.match(rustParser, /high_school_context_pattern\.is_match\(after\)/);
  assert.doesNotMatch(
    parser,
    /function extractCredits|function extractCourseDescription|function extractLabeledSectionHtml|function extractLabeledSectionText/
  );
  assert.equal(
    fs.existsSync(path.resolve(sourceRoot, "../automation/src/planner/grc_catalog.rs")),
    true
  );
});

test("GRC archive and current-catalog parsing are delegated to the offline Rust CLI", () => {
  const adapter = fs.readFileSync(path.join(__dirname, "grc-public-materials.cjs"), "utf8");
  const parserRunner =
    adapter.match(/function runRustGrcParser\(command, input, parserLabel\) \{([\s\S]*?)\n\}/)?.[1] ?? "";
  assert.match(parserRunner, /"--offline"/);
  assert.match(parserRunner, /"--locked"/);
  const archiveParser =
    adapter.match(/function extractGrcCatalogArchiveEntries\(html, baseUrl[^)]*\) \{([\s\S]*?)\n\}/)?.[1] ?? "";
  const currentParser =
    adapter.match(/function extractCurrentGrcCatalogDetails\([\s\S]*?\) \{([\s\S]*?)\n\}/)?.[1] ?? "";
  assert.match(archiveParser, /parse-grc-catalog-archive-entries/);
  assert.match(currentParser, /parse-grc-current-catalog-details/);
  assert.doesNotMatch(archiveParser, /extractAnchorMatches|new URL/);
  assert.doesNotMatch(currentParser, /extractAnchorMatches|new URL/);
});

test("UW-GRC equivalency rules and TypeScript output are generated by Rust", () => {
  const parser = fs.readFileSync(
    path.join(__dirname, "parse-transfer-planner-equivalency-guide.cjs"),
    "utf8"
  );
  const parseRows = parser.match(/function parseGuideRows\(html\) \{([\s\S]*?)\n\}/)?.[1] ?? "";
  assert.match(parseRows, /parse-equivalency-guide-rows/);
  assert.doesNotMatch(parseRows, /<h3|<table|<tr/);
  assert.match(parseRows, /grcCourseTitles: Object\.fromEntries\(GRC_COURSE_TITLE_BY_CODE\)/);
  assert.match(parser, /const rule = row\.generatedRule/);
  assert.match(parser, /return rule;/);
  const renderRules = parser.match(/function renderGeneratedRules\(scope\) \{([\s\S]*?)\n\}/)?.[1] ?? "";
  assert.match(renderRules, /render-equivalency-guide-typescript/);
  assert.match(renderRules, /input: JSON\.stringify\(scope\)/);
  assert.match(parser, /writeFileSync\(OUTPUT_TS_PATH, generatedSource\)/);
  assert.match(parser, /synthesize-uw-grc-derived-rules/);
  assert.match(parser, /supportRuleSelectors/);
  assert.match(parser, /build-equivalency-guide-report/);
  assert.match(parser, /writeReports\(\{ report, markdown \}\)/);
  assert.doesNotMatch(
    parser,
    /function buildDerivedRuleFromSpec|function createRuleLookup|function findBestSingleCourseRule|function findBestExactSourceSetRule|function buildCounts/
  );
  assert.doesNotMatch(parser, /function writeRuleChunks|JSON\.stringify\(chunk, null, 2\)/);
  assert.doesNotMatch(parser, /GRC_COURSE_TITLE_BY_CODE\.get/);
  assert.doesNotMatch(parser, /id: `uw-grc-guide:|sourceKind: "uw-green-river-equivalency-guide"/);
  assert.doesNotMatch(
    parser,
    /function parseSourceCourseSet|function parseTargetCourseCodes|function parseRequirementTags|function parseEffectiveYearRanges|function classifyRule|function buildAcceptanceCategory|function buildRuleStatus|function buildWarnings|function buildNotes|function slugify\(/
  );
  assert.equal(
    fs.existsSync(path.resolve(sourceRoot, "../automation/src/planner/equivalency_rows.rs")),
    true
  );
  assert.equal(
    fs.existsSync(path.resolve(sourceRoot, "../automation/src/planner/equivalency_derived.rs")),
    true
  );
  assert.equal(
    fs.existsSync(path.resolve(sourceRoot, "../automation/src/planner/equivalency_report.rs")),
    true
  );
});

test("Rust equivalency TypeScript renderer preserves the legacy JavaScript writer output", () => {
  const scope = {
    officialRules: [{ id: "guide:1", type: "direct-course", title: "Café & Biology", notes: ["QSR", "note"] }],
    derivedRules: [{ id: "derived:1", type: "sequence", weakerThanRuleIds: [] }],
  };
  const result = spawnSync(
    "cargo",
    [
      "run",
      "--offline",
      "--locked",
      "--quiet",
      "--manifest-path",
      "../automation/Cargo.toml",
      "--",
      "render-equivalency-guide-typescript",
    ],
    {
      cwd: sourceRoot,
      encoding: "utf8",
      input: JSON.stringify(scope),
      maxBuffer: 16 * 1024 * 1024,
    }
  );
  assert.equal(result.status, 0, result.stderr);

  const lines = [
    "/* eslint-disable */",
    "/* auto-generated by scripts/planner/parse-transfer-planner-equivalency-guide.cjs */",
    "",
    'import type { TransferPlannerEquivalencyRule } from "./schema";',
    "",
  ];
  const writeRuleChunks = (prefix, rules) => {
    const chunks = [];
    for (let index = 0; index < rules.length; index += 80) {
      chunks.push(rules.slice(index, index + 80));
    }
    chunks.forEach((chunk, index) => {
      lines.push(
        `const ${prefix}_CHUNK_${index}: unknown[] = ${JSON.stringify(chunk, null, 2)};`,
        ""
      );
    });
    const chunkNames = chunks.map((_, index) => `  ${prefix}_CHUNK_${index}`);
    lines.push(
      `const ${prefix}S_RAW: unknown[] = ([] as unknown[]).concat(`,
      chunkNames.join(",\n"),
      ");",
      ""
    );
  };
  writeRuleChunks("TRANSFER_PLANNER_UW_GRC_EQUIVALENCY_GUIDE_RULE", scope.officialRules);
  lines.push(
    "export const TRANSFER_PLANNER_UW_GRC_EQUIVALENCY_GUIDE_RULES =",
    "  TRANSFER_PLANNER_UW_GRC_EQUIVALENCY_GUIDE_RULES_RAW as TransferPlannerEquivalencyRule[];",
    ""
  );
  writeRuleChunks("TRANSFER_PLANNER_UW_GRC_DERIVED_EQUIVALENCY_RULE", scope.derivedRules);
  lines.push(
    "export const TRANSFER_PLANNER_UW_GRC_DERIVED_EQUIVALENCY_RULES =",
    "  TRANSFER_PLANNER_UW_GRC_DERIVED_EQUIVALENCY_RULES_RAW as TransferPlannerEquivalencyRule[];",
    "",
    "export const TRANSFER_PLANNER_UW_GRC_ALL_EQUIVALENCY_RULES =",
    "  ([] as TransferPlannerEquivalencyRule[]).concat(",
    "    TRANSFER_PLANNER_UW_GRC_EQUIVALENCY_GUIDE_RULES,",
    "    TRANSFER_PLANNER_UW_GRC_DERIVED_EQUIVALENCY_RULES",
    "  );",
    ""
  );
  assert.equal(result.stdout, `${lines.join("\n")}\n`);
});

test("Rust renderer matches the tracked equivalency artifact after checkout line-ending normalization", () => {
  require("ts-node").register({
    skipProject: true,
    transpileOnly: true,
    compilerOptions: { module: "CommonJS", moduleResolution: "node" },
  });
  const generatedPath = path.join(
    sourceRoot,
    "constants/transfer-planner-source/equivalency-guide.generated.ts"
  );
  const generated = require(generatedPath);
  const scope = {
    officialRules: generated.TRANSFER_PLANNER_UW_GRC_EQUIVALENCY_GUIDE_RULES,
    derivedRules: generated.TRANSFER_PLANNER_UW_GRC_DERIVED_EQUIVALENCY_RULES,
  };
  const result = spawnSync(
    "cargo",
    [
      "run",
      "--offline",
      "--locked",
      "--quiet",
      "--manifest-path",
      "../automation/Cargo.toml",
      "--",
      "render-equivalency-guide-typescript",
    ],
    {
      cwd: sourceRoot,
      encoding: "utf8",
      input: JSON.stringify(scope),
      maxBuffer: 16 * 1024 * 1024,
    }
  );
  assert.equal(result.status, 0, result.stderr);
  const trackedArtifact = fs.readFileSync(generatedPath, "utf8").replace(/\r\n/g, "\n");
  assert.equal(result.stdout, trackedArtifact);
});
