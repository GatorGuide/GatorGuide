/* Read-only adapter for canonical TypeScript planner data consumed by the Rust source-pipeline audit. */
const fs = require("fs");
const path = require("path");

const sourceRoot = path.resolve(process.argv[2] || process.cwd());
const tsNodePath = require.resolve("ts-node", { paths: [sourceRoot] });
require(tsNodePath).register({
  skipProject: true,
  transpileOnly: true,
  compilerOptions: {
    module: "CommonJS",
    moduleResolution: "node",
  },
});

const sourceModule = require(path.join(sourceRoot, "constants/transfer-planner-source"));
const { TRANSFER_PLANNER_PRIMARY_PROMOTIONS } = require(path.join(
  sourceRoot,
  "constants/transfer-planner-source/primary-source-promotions.generated"
));
const { TRANSFER_PLANNER_REQUIREMENT_FINGERPRINTS } = require(path.join(
  sourceRoot,
  "constants/transfer-planner-source/source-fingerprints.generated"
));
const { TRANSFER_PLANNER_PARSED_REQUIREMENT_BLOCKS } = require(path.join(
  sourceRoot,
  "constants/transfer-planner-source/requirement-source-adapters.generated"
));
const { TRANSFER_PLANNER_MANUAL_LINK_OVERRIDES } = require(path.join(
  sourceRoot,
  "constants/transfer-planner-source/manual-source-link-overrides.data"
));
const {
  buildTransferPlannerOwnerId,
  normalizeTransferPlannerOwnerId,
} = require(path.join(
  sourceRoot,
  "constants/transfer-planner-source/pathway-id-normalization"
));
const {
  isSuspiciousStructuralPathwayId,
  isSuspiciousStructuralPathwayLabel,
} = require(path.join(
  sourceRoot,
  "constants/transfer-planner-source/pathway-materialization"
));
const { getParseablePrimaryEntries } = require(path.join(
  sourceRoot,
  "scripts/planner/parse-transfer-planner-requirement-sources.cjs"
));
const { getTransferPlannerStudentRuntimeAliasCoverage } = require(path.join(
  sourceRoot,
  "constants/transfer-planner-source/student-runtime"
));

function readInput() {
  let raw = "";
  try {
    raw = fs.readFileSync(0, "utf8");
    return raw.trim() ? JSON.parse(raw) : {};
  } catch (error) {
    throw new Error(`could not decode source-pipeline bridge input: ${error.message}`);
  }
}

function reviewQueueEntries(reviewQueue) {
  return (reviewQueue?.campuses ?? []).flatMap((campus) => campus?.entries ?? []);
}

function collectRuntimeAliasOwnerIds(input) {
  const candidates = [
    ...(input.discoveryReport?.owners ?? []),
    ...(input.discoveryReport?.weakExistingOwners ?? []),
    ...reviewQueueEntries(input.reviewQueue),
  ];
  const ids = new Set();
  for (const owner of candidates) {
    if (!owner?.planId) continue;
    if (!getTransferPlannerStudentRuntimeAliasCoverage(owner.planId, owner.pathwayId ?? null)) {
      continue;
    }
    ids.add(buildTransferPlannerOwnerId(owner.planId, owner.pathwayId ?? null));
  }
  return [...ids];
}

function collectRuntimePathwayRecords(plans) {
  return plans.flatMap((plan) => {
    const runtimePlan = sourceModule.getTransferPlannerStudentRuntimeMajorPlan(plan.id);
    if (!runtimePlan) return [];
    return [{
      planId: plan.id,
      pathways: sourceModule.getTransferPlannerStudentRuntimePathwaysForPlan(runtimePlan),
    }];
  });
}

function collectSuspiciousPathwayOwnerKeys(pathways) {
  return pathways
    .filter((entry) =>
      isSuspiciousStructuralPathwayId(entry?.pathwayId) ||
      isSuspiciousStructuralPathwayLabel(entry?.label)
    )
    .map((entry) =>
      normalizeTransferPlannerOwnerId(entry.id, entry.planId, entry.pathwayId)
    );
}

function main() {
  const input = readInput();
  const manifest = sourceModule.TRANSFER_PLANNER_MANIFEST_REGISTRY ?? [];
  const plans = sourceModule.TRANSFER_PLANNER_GENERATED_MAJOR_PLANS ?? [];
  const pathways = sourceModule.TRANSFER_PLANNER_MAJOR_PATHWAY_REGISTRY ?? [];
  const parseableParserTypes = new Set([
    "html-degree-page",
    "html-curriculum-page",
    "html-overview-page",
    "catalog-page",
    "generic-html",
    "pdf-degree-sheet",
    "pdf-worksheet",
    "generic-pdf",
  ]);
  const parseablePrimaryManifest = manifest.filter(
    (entry) =>
      (entry.ownerType === "major" || entry.ownerType === "pathway") &&
      entry.campusId &&
      entry.campusId !== "grc" &&
      entry.isPrimaryDegreeRequirementsLink &&
      parseableParserTypes.has(entry.parserType)
  );

  process.stdout.write(JSON.stringify({
    canonicalPrimaryManifest: manifest,
    parseablePrimaryManifest,
    parseableRequirementSourceEntries: getParseablePrimaryEntries(),
    promotions: TRANSFER_PLANNER_PRIMARY_PROMOTIONS ?? [],
    generatedParsedBlocks: TRANSFER_PLANNER_PARSED_REQUIREMENT_BLOCKS ?? [],
    generatedRequirementFingerprints: TRANSFER_PLANNER_REQUIREMENT_FINGERPRINTS ?? [],
    activePathways: pathways,
    generatedPlans: plans,
    manualOverrides: TRANSFER_PLANNER_MANUAL_LINK_OVERRIDES ?? [],
    runtimeAliasCoveredOwnerIds: collectRuntimeAliasOwnerIds(input),
    runtimePathwayRecords: collectRuntimePathwayRecords(plans),
    structurallySuspiciousOwnerKeys: collectSuspiciousPathwayOwnerKeys(pathways),
  }));
}

try {
  main();
} catch (error) {
  console.error(error?.stack || error);
  process.exitCode = 1;
}
