const assert = require("node:assert/strict");
const test = require("node:test");

const {
  parseSequenceCourseCodes,
  parseSequenceInlineSegments,
} = require("./lib/rust-sequence-parser.cjs");

test("Rust sequence parser carries course subjects without mixing explicit subjects", (context) => {
  const actual = parseSequenceCourseCodes([
    {
      text: "MATH 124, 125, 126",
      fallbackSubject: "MATH",
      explicitCourseCodes: ["MATH 124"],
    },
    {
      text: "PHYS 122 or CSE 122",
      fallbackSubject: "PHYS",
      explicitCourseCodes: ["CSE 122", "PHYS 122"],
    },
  ]);
  if (!actual) {
    context.skip("offline Rust automation executable is unavailable");
    return;
  }

  assert.deepEqual(actual, [
    ["MATH 124", "MATH 125", "MATH 126"],
    ["CSE 122", "PHYS 122"],
  ]);
});

test("Rust inline sequence parser batches and preserves alternative order", (context) => {
  const actual = parseSequenceInlineSegments([
    "PHYS 121 + 122 + 123 -OR- PHYS 114 + 115 + 116",
    "Choose one sequence: MATH 124, 125, 126; or MATH 134, 135, 136",
  ]);
  if (!actual) {
    context.skip("offline Rust automation executable is unavailable");
    return;
  }

  assert.deepEqual(actual, [
    ["PHYS 121, 122, 123", "PHYS 114, 115, 116"],
    ["Choose one sequence: MATH 124, 125, 126", "MATH 134, 135, 136"],
  ]);
});
