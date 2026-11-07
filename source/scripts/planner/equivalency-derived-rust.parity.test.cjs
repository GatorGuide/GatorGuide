const assert = require("node:assert/strict");
const test = require("node:test");

process.env.TS_NODE_COMPILER_OPTIONS = JSON.stringify({
  module: "Node16",
  moduleResolution: "Node16",
});
require("ts-node").register({ transpileOnly: true });

const generated = require("../../constants/transfer-planner-source/equivalency-guide.generated.ts");
const {
  buildDerivedChainRules,
  buildDerivedStructuredEquivalencyRules,
} = require("./parse-transfer-planner-equivalency-guide.cjs");

const officialRules = generated.TRANSFER_PLANNER_UW_GRC_EQUIVALENCY_GUIDE_RULES;
const expectedDerivedRules = generated.TRANSFER_PLANNER_UW_GRC_DERIVED_EQUIVALENCY_RULES;

test("Rust derived-rule synthesis preserves the tracked UW-GRC derived-rule artifact", () => {
  const structuredRules = buildDerivedStructuredEquivalencyRules(officialRules);
  const chainRules = buildDerivedChainRules(officialRules, structuredRules);
  const actualDerivedRules = [...structuredRules, ...chainRules].sort((left, right) =>
    left.id.localeCompare(right.id)
  );

  assert.equal(actualDerivedRules.length, expectedDerivedRules.length);
  assert.deepStrictEqual(actualDerivedRules, expectedDerivedRules);
});
