const assert = require("node:assert/strict");
const test = require("node:test");

require("../../../scripts/qa/react-native-test-utils.cjs");

const { RESOURCE_CATALOG } = require("@/constants/resource-catalog");

test("Resources links the 2026 Allen School transfer rubric from Transfer Guides", () => {
  const transferGuides = RESOURCE_CATALOG
    .find((section) => section.id === "student-tools")
    ?.subsections?.find((subsection) => subsection.id === "transfer-guides");
  const rubricItems = transferGuides?.items.filter(
    (item) => item.title === "Allen School Transfer Application Rubric (2026)"
  );

  assert.equal(rubricItems?.length, 1);
  assert.equal(
    rubricItems?.[0].url,
    "https://www.cs.washington.edu/wp-content/uploads/2025/11/Spring-2026-Admissions-Rubric-Allen-School-Admissions.pdf"
  );
});
