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

function makeOpportunity({ opportunityId, title, type, listingKind = "individual" }) {
  const dueAt = new Date();
  dueAt.setDate(dueAt.getDate() + 30);

  return {
    schemaVersion: 1,
    opportunityId,
    type,
    listingKind,
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

function visibleTextOrder(renderer) {
  return renderer.root
    .findAll((node) => node.type === "Text")
    .map((node) => node.props.children)
    .filter((value) => typeof value === "string");
}

function completionCheckboxLabels(renderer) {
  return renderer.root
    .findAll((node) => node.type === "Pressable" && node.props.accessibilityRole === "checkbox")
    .map((node) => node.props.accessibilityLabel);
}

test("Resources places Deadlines before Career Links and AI Resources below STEM Clubs @ GRC", () => {
  resetReactNativeTestState();
  setTranslations({ "resources.deadlines": "Deadlines" });
  setOpportunities({
    isHydrated: true,
    matchedOpportunities: [
      makeOpportunity({
        opportunityId: "grc-deadline",
        title: "GRC enrollment deadline",
        type: "college_deadline",
      }),
    ],
    refreshOpportunitiesIfNeeded: createSpy(async () => undefined),
    setOpportunityDone: createSpy(async () => undefined),
  });

  const renderer = render(React.createElement(ResourcesPage));
  const labels = visibleTextOrder(renderer);
  const deadlinesIndex = labels.indexOf("Deadlines");
  const stemClubsIndex = labels.indexOf("STEM Clubs @ GRC");
  const aiResourcesIndex = labels.indexOf("AI Resources");
  const careerLinksIndex = labels.indexOf("Career Links");

  assert.ok(deadlinesIndex >= 0 && deadlinesIndex < careerLinksIndex);
  assert.ok(stemClubsIndex >= 0 && stemClubsIndex < aiResourcesIndex);

  unmount(renderer);
});

test("Resources combines college and academic dates under GRC Deadlines", () => {
  resetReactNativeTestState();
  setTranslations({
    "resources.deadlines": "Deadlines",
    "resources.collegeDeadlines": "GRC Deadlines",
    "resources.generalDeadlines": "General deadlines",
    "resources.academicCalendar": "Academic calendar",
  });
  setOpportunities({
    isHydrated: true,
    matchedOpportunities: [
      makeOpportunity({
        opportunityId: "grc-enrollment-deadline",
        title: "GRC enrollment deadline",
        type: "college_deadline",
      }),
      makeOpportunity({
        opportunityId: "grc-quarter-start",
        title: "GRC quarter starts",
        type: "quarter-start",
      }),
      makeOpportunity({
        opportunityId: "grc-quarter-end",
        title: "GRC quarter ends",
        type: "quarter-end",
      }),
      makeOpportunity({
        opportunityId: "fafsa-deadline",
        title: "Federal FAFSA deadline",
        type: "general_deadline",
      }),
    ],
    refreshOpportunitiesIfNeeded: createSpy(async () => undefined),
    setOpportunityDone: createSpy(async () => undefined),
  });

  const renderer = render(React.createElement(ResourcesPage));
  pressByText(renderer, "Deadlines");
  pressByText(renderer, "GRC Deadlines");

  assert.equal(hasText(renderer, "GRC enrollment deadline"), true);
  assert.equal(hasText(renderer, "GRC quarter starts"), true);
  assert.equal(hasText(renderer, "GRC quarter ends"), true);
  assert.equal(hasText(renderer, "Academic calendar"), false);
  assert.equal(hasText(renderer, "Federal FAFSA deadline"), false);

  pressByText(renderer, "General deadlines");
  assert.equal(hasText(renderer, "Federal FAFSA deadline"), true);

  unmount(renderer);
});

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

test("Resources hides completion checkboxes for scholarship and internship databases", () => {
  resetReactNativeTestState();
  setTranslations({
    "resources.scholarships": "Scholarships",
    "resources.internships": "Internships",
    "resources.individualScholarships": "Individual scholarships",
    "resources.individualInternships": "Individual internships",
    "resources.actionMarkDone": "Mark done",
    "resources.actionUndo": "Undo",
  });
  setOpportunities({
    isHydrated: true,
    matchedOpportunities: [
      makeOpportunity({
        opportunityId: "scholarship-database",
        title: "Scholarship database",
        type: "scholarship",
        listingKind: "database",
      }),
      makeOpportunity({
        opportunityId: "individual-scholarship",
        title: "Individual scholarship",
        type: "scholarship",
      }),
      makeOpportunity({
        opportunityId: "internship-database",
        title: "Internship database",
        type: "internship",
        listingKind: "database",
      }),
      makeOpportunity({
        opportunityId: "individual-internship",
        title: "Individual internship",
        type: "internship",
      }),
    ],
    refreshOpportunitiesIfNeeded: createSpy(async () => undefined),
    setOpportunityDone: createSpy(async () => undefined),
  });

  const renderer = render(React.createElement(ResourcesPage));
  pressByText(renderer, "Scholarships");
  pressByText(renderer, "Databases");
  assert.equal(hasText(renderer, "Scholarship database"), true);
  assert.deepEqual(completionCheckboxLabels(renderer), []);

  pressByText(renderer, "Individual scholarships");
  assert.deepEqual(completionCheckboxLabels(renderer), ["Mark done: Individual scholarship"]);

  pressByText(renderer, "Scholarships");
  pressByText(renderer, "Internships");
  pressByText(renderer, "Databases");
  assert.equal(hasText(renderer, "Internship database"), true);
  assert.deepEqual(completionCheckboxLabels(renderer), []);

  pressByText(renderer, "Individual internships");
  assert.deepEqual(completionCheckboxLabels(renderer), ["Mark done: Individual internship"]);

  unmount(renderer);
});
