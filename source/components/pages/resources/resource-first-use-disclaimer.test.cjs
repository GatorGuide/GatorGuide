const assert = require("node:assert/strict");
const Module = require("node:module");
const test = require("node:test");

const testUtils = require("../../../scripts/qa/react-native-test-utils.cjs");
const {
  React,
  act,
  createSpy,
  hasText,
  pressByText,
  render,
  resetReactNativeTestState,
  setOpportunities,
  testState,
  unmount,
} = testUtils;

const { STORAGE_KEYS } = require("@/constants/schema");
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

const DISCLAIMER =
  "Use everything in this database at your own discretion. Unofficial tool for Green River College students).";

async function flushStorageRead() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

function prepareResourcesPage() {
  setOpportunities({
    isHydrated: true,
    matchedOpportunities: [],
    refreshOpportunitiesIfNeeded: createSpy(async () => undefined),
    setOpportunityDone: createSpy(async () => undefined),
  });
  return render(React.createElement(ResourcesPage));
}

test("first-use resource disclaimer requires OK once and persists acknowledgment", async () => {
  resetReactNativeTestState();
  const renderer = prepareResourcesPage();
  await flushStorageRead();

  assert.equal(hasText(renderer, DISCLAIMER), true);
  assert.equal(hasText(renderer, "OK"), true);
  assert.equal(testState.localStorage.has(STORAGE_KEYS.resourceDisclaimerAcknowledged), false);

  const modal = renderer.root.findByType("Modal");
  act(() => modal.props.onRequestClose?.());
  assert.equal(hasText(renderer, DISCLAIMER), true);

  pressByText(renderer, "OK");
  await act(async () => {
    await Promise.resolve();
  });
  assert.equal(
    testState.localStorage.get(STORAGE_KEYS.resourceDisclaimerAcknowledged),
    "true"
  );
  assert.equal(hasText(renderer, DISCLAIMER), false);
  unmount(renderer);

  const returningRenderer = prepareResourcesPage();
  await flushStorageRead();
  assert.equal(hasText(returningRenderer, DISCLAIMER), false);
  assert.equal(returningRenderer.root.findAllByType("Modal").length, 0);
  unmount(returningRenderer);
});
