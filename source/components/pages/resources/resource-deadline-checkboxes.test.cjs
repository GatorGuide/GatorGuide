const assert = require("node:assert/strict");
const Module = require("node:module");
const test = require("node:test");

const testUtils = require("../../../scripts/qa/react-native-test-utils.cjs");
const {
  React,
  createSpy,
  hasText,
  pressByText,
  render,
  resetReactNativeTestState,
  setOpportunities,
  setTranslations,
  unmount,
} = testUtils;

const originalModuleLoad = Module._load;
Module._load = function loadWithFocusEffect(request, parent, isMain) {
  const loaded = originalModuleLoad.call(this, request, parent, isMain);
  if (request !== "expo-router") return loaded;
  return {
    ...loaded,
    useFocusEffect: (callback) => React.useEffect(callback, [callback]),
  };
};
const ResourcesPage = require("@/components/pages/ResourcesPage").default;
Module._load = originalModuleLoad;

function makeOpportunity({ opportunityId, title, type }) {
  const dueAt = new Date();
  dueAt.setDate(dueAt.getDate() + 30);

  return {
    schemaVersion: 1,
    opportunityId,
    type,
    listingKind: "individual",
    status: "active",
    title,
    organizationName: "GatorGuide",
    summary: `${title} details.`,
    externalUrl: null,
    dueAt: dueAt.toISOString(),
    recurrence: { isYearly: false, month: null, day: null, timezone: null },
    deadline: { type: "final", label: null },
    matching: { financialAidTags: [], suggestedMajors: [], hasToBeMajor: false },
    eligibility: { gpaMin: null, residencyTypes: [], communityTags: [], transferOnly: false },
    requirements: { needsRecommendations: false, recommendationCountMin: 0, essayCount: 0 },
    award: {
      amountMin: null,
      amountMax: null,
      currency: "USD",
      amountText: null,
      renewable: null,
    },
    college: {
      collegeId: null,
      collegeName: null,
      city: null,
      state: null,
      website: null,
    },
    source: {
      kind: "manual",
      sourceUrl: null,
      sourceLabel: null,
      model: null,
      fetchedAt: null,
      verifiedAt: null,
    },
    createdAt: null,
    updatedAt: null,
    computedDueAt: dueAt.toISOString(),
    progress: null,
    isDone: false,
    matchScore: 100,
    matchReasons: [],
  };
}

test("Resources hides completion checkboxes for every deadline but retains them for other opportunities", () => {
  resetReactNativeTestState();
  setTranslations({
    "resources.deadlines": "Deadlines",
    "resources.generalDeadlines": "General deadlines",
    "resources.individualScholarships": "Individual scholarships",
    "resources.scholarships": "Scholarships",
    "resources.actionMarkDone": "Mark done",
    "resources.actionUndo": "Undo",
  });
  setOpportunities({
    isHydrated: true,
    matchedOpportunities: [
      makeOpportunity({
        opportunityId: "application-deadline",
        title: "Application deadline",
        type: "general_deadline",
      }),
      makeOpportunity({
        opportunityId: "promise-scholarship",
        title: "Promise Scholarship",
        type: "scholarship",
      }),
    ],
    refreshOpportunitiesIfNeeded: createSpy(async () => undefined),
    setOpportunityDone: createSpy(async () => undefined),
  });

  const renderer = render(React.createElement(ResourcesPage));
  pressByText(renderer, "Deadlines");
  pressByText(renderer, "General deadlines");

  assert.equal(hasText(renderer, "Application deadline"), true);
  assert.deepEqual(
    renderer.root
      .findAll((node) => node.type === "Pressable" && node.props.accessibilityRole === "checkbox")
      .map((node) => node.props.accessibilityLabel),
    []
  );

  pressByText(renderer, "Scholarships");
  pressByText(renderer, "Individual scholarships");
  assert.deepEqual(
    renderer.root
      .findAll((node) => node.type === "Pressable" && node.props.accessibilityRole === "checkbox")
      .map((node) => node.props.accessibilityLabel),
    ["Mark done: Promise Scholarship"]
  );

  unmount(renderer);
});
