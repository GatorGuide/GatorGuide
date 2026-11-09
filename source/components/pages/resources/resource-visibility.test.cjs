const assert = require("node:assert/strict");
const Module = require("node:module");
const test = require("node:test");

const testUtils = require("../../../scripts/qa/react-native-test-utils.cjs");
const {
  React,
  createSpy,
  changeTextByPlaceholder,
  hasText,
  pressByText,
  render,
  resetReactNativeTestState,
  setOpportunities,
  setTranslations,
  testState,
  unmount,
} = testUtils;

const { STORAGE_KEYS } = require("@/constants/schema");
const { RESOURCE_CATALOG } = require("@/constants/resource-catalog");
const originalModuleLoad = Module._load;
Module._load = function loadWithFocusEffect(request, parent, isMain) {
  const loaded = originalModuleLoad.call(this, request, parent, isMain);
  if (request !== "expo-router") return loaded;
  return {
    ...loaded,
    useFocusEffect: (callback) => React.useEffect(callback, [callback]),
  };
};
const {
  isTransferPlannerResourceUrl,
  TRANSFER_PLANNER_EXPERIMENTAL_PREFIX,
} = require("@/components/pages/resources/useResourcesPageModel");
const ResourcesPage = require("@/components/pages/ResourcesPage").default;
Module._load = originalModuleLoad;

function getCatalogItems() {
  return RESOURCE_CATALOG.flatMap((section) => [
    ...(section.items ?? []),
    ...(section.subsections ?? []).flatMap((subsection) => subsection.items),
  ]);
}

test("Transfer Planner is visible, searchable, and starts with its experimental warning", () => {
  resetReactNativeTestState();
  setTranslations({
    "resources.actionOpen": "Open",
    "resources.searchPlaceholder": "Search resources",
    "resources.tools": "Tools",
    "resources.transferPlanner": "Transfer Planner",
    "resources.transferPlannerDesc": "Find your degree path with track matching and classes that directly transfer over.",
  });
  setOpportunities({
    isHydrated: true,
    matchedOpportunities: [],
    refreshOpportunitiesIfNeeded: createSpy(async () => undefined),
    setOpportunityDone: createSpy(async () => undefined),
  });
  testState.localStorage.set(STORAGE_KEYS.resourceDisclaimerAcknowledged, "true");

  const catalogItems = getCatalogItems();
  const plannerItems = catalogItems.filter((item) => isTransferPlannerResourceUrl(item.url));
  assert.equal(plannerItems.length, 1);
  assert.equal(plannerItems[0].titleKey, "resources.transferPlanner");
  assert.equal(plannerItems[0].descriptionKey, "resources.transferPlannerDesc");

  const renderer = render(React.createElement(ResourcesPage));
  pressByText(renderer, "Tools");

  const expectedDescription = `${TRANSFER_PLANNER_EXPERIMENTAL_PREFIX} Find your degree path with track matching and classes that directly transfer over.`;
  assert.equal(hasText(renderer, "Transfer Planner"), true);
  assert.equal(hasText(renderer, expectedDescription), true);

  changeTextByPlaceholder(renderer, "Search resources", "transfer planner");
  assert.equal(hasText(renderer, "Transfer Planner"), true);
  assert.equal(hasText(renderer, expectedDescription), true);

  unmount(renderer);
});
