const assert = require("node:assert/strict");
const test = require("node:test");

const {
  React,
  act,
  createSpy,
  getRouter,
  render,
  resetReactNativeTestState,
  setAppData,
  unmount,
} = require("../scripts/qa/react-native-test-utils.cjs");

const { ROUTES } = require("@/constants/routes");
const Index = require("@/app/index").default;

async function waitForStartupNavigation() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 200));
  });
}

async function navigateFromIndex(user) {
  resetReactNativeTestState();
  const signInAsGuest = createSpy(async () => undefined);
  setAppData({
    isHydrated: true,
    state: { user },
    signInAsGuest,
  });

  const renderer = render(React.createElement(Index));
  await waitForStartupNavigation();
  const replaceCalls = [...getRouter().replace.calls];
  unmount(renderer);
  return { replaceCalls, signInAsGuest };
}

test("Index signs first-time unauthenticated users in as guests and opens Resources", async () => {
  const { replaceCalls, signInAsGuest } = await navigateFromIndex(null);

  assert.equal(signInAsGuest.calls.length, 1);
  assert.deepEqual(replaceCalls, [[ROUTES.tabsResources]]);
});

test("Index routes guests and authenticated users with completed setup to Resources", async () => {
  const guestResult = await navigateFromIndex({ uid: "guest-1", isGuest: true });
  const authenticatedResult = await navigateFromIndex({
    uid: "student-1",
    isGuest: false,
    isProfileComplete: true,
  });

  assert.deepEqual(guestResult.replaceCalls, [[ROUTES.tabsResources]]);
  assert.deepEqual(authenticatedResult.replaceCalls, [[ROUTES.tabsResources]]);
  assert.equal(guestResult.signInAsGuest.calls.length, 0);
  assert.equal(authenticatedResult.signInAsGuest.calls.length, 0);
});

test("Index preserves profile setup for authenticated users who have not completed setup", async () => {
  const { replaceCalls, signInAsGuest } = await navigateFromIndex({
    uid: "student-2",
    isGuest: false,
  });

  assert.deepEqual(replaceCalls, [[ROUTES.profileSetup]]);
  assert.equal(signInAsGuest.calls.length, 0);
});
