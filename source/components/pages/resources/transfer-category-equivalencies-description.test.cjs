const assert = require("node:assert/strict");
const test = require("node:test");

require("../../../scripts/qa/react-native-test-utils.cjs");

const englishResources = require("../../../constants/locales/en.json").resources;
const {
  splitResourceDescriptionAtLink,
  TRANSFER_CATEGORY_EQUIVALENCY_GUIDE_URL,
} = require("@/components/pages/resources/useResourcesPageModel");

test("Transfer Category Equivalencies shows its guide note, refresh date, and linkable label", () => {
  const description = englishResources.transferCategoryEquivalenciesDescription;
  const linkLabel = englishResources.transferCategoryEquivalenciesGuideLabel;
  const parts = splitResourceDescriptionAtLink(description, linkLabel);

  assert.equal(
    description.split("\n")[0],
    "Eligible Green River to UW Seattle transfer options for A&H (Arts and Humanities), SSc (Social Sciences), NSc (Natural Sciences), CE-approved Natural Science, and related requirement categories."
  );
  assert.equal(linkLabel, "Equivalency Guide");
  assert.equal(parts.before, `${description.split("\n")[0]}\nDouble check using `);
  assert.match(
    parts.after,
    /^\. Last updated at (?:____|[A-Z][a-z]+ [0-9]{1,2}, [0-9]{4})\.$/
  );
  assert.equal(
    TRANSFER_CATEGORY_EQUIVALENCY_GUIDE_URL,
    "https://admit.washington.edu/apply/transfer/equivalency-guide/green-river/"
  );
});
