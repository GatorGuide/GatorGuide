const assert = require("node:assert/strict");
const test = require("node:test");

require("../../../scripts/qa/react-native-test-utils.cjs");

const { RESOURCE_CATALOG } = require("@/constants/resource-catalog");

const clubDirectoryUrl =
  "https://www.greenriver.edu/students/get-involved/clubs-and-organizations.html";
const expectedClubs = [
  "Women in STEM",
  "Engineering Club",
  "Artificial Intelligence (AI) Club",
  "CyberOps Club",
  "Math Club",
  "Physics Club",
  "Chemistry Club",
];

test("STEM Clubs @ GRC lists the official Green River College clubs", () => {
  const section = RESOURCE_CATALOG.find((entry) => entry.id === "stem-clubs-grc");

  assert.ok(section, "STEM Clubs @ GRC resource section exists");
  assert.equal(section.title, "STEM Clubs @ GRC");
  assert.deepEqual(
    section.items.map((item) => item.title),
    expectedClubs
  );

  for (const item of section.items) {
    assert.equal(item.url, clubDirectoryUrl);
    assert.ok(item.description);
    assert.ok(item.tags?.includes("STEM Clubs @ GRC"));
  }
});
