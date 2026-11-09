const assert = require("node:assert/strict");
const test = require("node:test");

const {
  React,
  getRouter,
  render,
  resetReactNativeTestState,
  setLocalSearchParams,
  unmount,
} = require("../scripts/qa/react-native-test-utils.cjs");
const { ROUTES } = require("@/constants/routes");
const Login = require("@/app/login").default;

test("Login route sends ordinary visits to Resources instead of showing the auth screen", () => {
  resetReactNativeTestState();
  setLocalSearchParams({});

  const renderer = render(React.createElement(Login));

  assert.deepEqual(getRouter().replace.calls, [[ROUTES.tabsResources]]);
  assert.equal(renderer.root.findAllByType("TextInput").length, 0);
  unmount(renderer);
});

test("Login route also sends explicit account actions to Resources", () => {
  resetReactNativeTestState();
  setLocalSearchParams({ intent: "account" });

  const renderer = render(React.createElement(Login));

  assert.deepEqual(getRouter().replace.calls, [[ROUTES.tabsResources]]);
  assert.equal(renderer.root.findAllByType("TextInput").length, 0);
  unmount(renderer);
});
