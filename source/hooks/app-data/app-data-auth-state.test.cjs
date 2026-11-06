require("ts-node").register({
  skipProject: true,
  transpileOnly: true,
  compilerOptions: {
    module: "CommonJS",
    moduleResolution: "node",
    jsx: "react-jsx",
    baseUrl: ".",
    paths: { "@/*": ["./*"] },
  },
});
require("tsconfig-paths/register");

const assert = require("node:assert/strict");
const test = require("node:test");
const { initialState } = require("@/hooks/app-data/app-data-state");
const { resolveAppDataForAccount } = require("@/hooks/app-data/app-data-auth-state");

const authUser = { uid: "account-b", email: "b@example.com", name: "Account B" };

test("first sign-in promotes the current guest snapshot into a local account", () => {
  const guestState = {
    ...initialState,
    user: {
      uid: "guest-1",
      email: "guest-1@gatorguide.local",
      name: "Guest User",
      isGuest: true,
      major: "biology",
      gpa: "3.4",
    },
    questionnaireAnswers: { goal: "transfer" },
    savedColleges: [{ id: "college-1", name: "Local College" }],
  };

  const result = resolveAppDataForAccount(guestState, null, authUser);

  assert.equal(result.user.uid, authUser.uid);
  assert.equal(result.user.isGuest, false);
  assert.equal(result.user.major, "biology");
  assert.deepEqual(result.questionnaireAnswers, guestState.questionnaireAnswers);
  assert.deepEqual(result.savedColleges, guestState.savedColleges);
});

test("an existing local account snapshot wins over unrelated guest data", () => {
  const guestState = {
    ...initialState,
    user: {
      uid: "guest-1",
      email: "guest-1@gatorguide.local",
      name: "Guest User",
      isGuest: true,
      major: "biology",
    },
    questionnaireAnswers: { privateAnswer: "guest-only" },
  };
  const accountState = {
    ...initialState,
    user: {
      uid: "account-b",
      email: "old-b@example.com",
      name: "Saved B",
      isGuest: false,
      major: "nursing",
    },
    questionnaireAnswers: { privateAnswer: "account-b" },
  };

  const result = resolveAppDataForAccount(guestState, accountState, authUser);

  assert.equal(result.user.major, "nursing");
  assert.deepEqual(result.questionnaireAnswers, { privateAnswer: "account-b" });
});

test("switching accounts without a local snapshot does not leak the previous account state", () => {
  const accountA = {
    ...initialState,
    user: {
      uid: "account-a",
      email: "a@example.com",
      name: "Account A",
      isGuest: false,
      major: "physics",
    },
    questionnaireAnswers: { privateAnswer: "account-a" },
  };

  const result = resolveAppDataForAccount(accountA, null, authUser);

  assert.equal(result.user.uid, authUser.uid);
  assert.equal(result.user.major, "");
  assert.deepEqual(result.questionnaireAnswers, {});
});
