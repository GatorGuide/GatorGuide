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
const ResourcesPage = require("@/components/pages/ResourcesPage").default;
Module._load = originalModuleLoad;

test("AI Resources lists the verified offers, official sources, and claim links", () => {
  const section = RESOURCE_CATALOG.find((entry) => entry.id === "ai-resources");
  assert.ok(section);
  assert.equal(section.title, "AI Resources");
  assert.deepEqual(
    section.items.map(({ title, actionLabel, url, sourceLabel, sourceUrl }) => ({
      title,
      actionLabel,
      url,
      sourceLabel,
      sourceUrl,
    })),
    [
      {
        title: "Google AI Pro Student Trial",
        actionLabel: "Claim Google AI student offer",
        url: "https://one.google.com/ai-student?plans=1%2C2",
        sourceLabel: "Google Help",
        sourceUrl: "https://gemini.google/students/",
      },
      {
        title: "OpenAI Codex for Students",
        actionLabel: "Claim Codex student credits",
        url: "https://chatgpt.com/codex/offers/students",
        sourceLabel: "OpenAI Developers",
        sourceUrl: "https://chatgpt.com/codex/students/",
      },
      {
        title: "GitHub Copilot Student",
        actionLabel: "GitHub Student Developer Pack",
        url: "https://education.github.com/pack",
        sourceLabel: "GitHub Education",
        sourceUrl: "https://github.com/education/students",
      },
    ]
  );

  const [google, codex, copilot] = section.items;
  assert.match(google.description, /12 months/);
  assert.match(google.description, /SheerID/);
  assert.match(google.description, /valid school email/);
  assert.match(google.description, /personal Google account/);
  assert.match(google.description, /qualifying payment method/);
  assert.match(codex.description, /\$100/);
  assert.match(codex.description, /2,500 ChatGPT credits/);
  assert.match(codex.description, /U\.S\. or Canada/);
  assert.match(codex.description, /12 months/);
  assert.match(codex.description, /not API credits/);
  assert.match(copilot.description, /unlimited completions/);
  assert.match(copilot.description, /GitHub AI Credits/);
  assert.match(copilot.description, /limited chat and agent usage/);

  resetReactNativeTestState();
  setTranslations({ "resources.actionOpen": "Open" });
  setOpportunities({
    isHydrated: true,
    matchedOpportunities: [],
    refreshOpportunitiesIfNeeded: createSpy(async () => undefined),
    setOpportunityDone: createSpy(async () => undefined),
  });

  const renderer = render(React.createElement(ResourcesPage));
  pressByText(renderer, "AI Resources");
  for (const label of [
    "Google AI Pro Student Trial",
    "OpenAI Codex for Students",
    "GitHub Copilot Student",
    "Claim Google AI student offer",
    "Claim Codex student credits",
    "GitHub Student Developer Pack",
    "Source: Google Help",
    "Source: OpenAI Developers",
    "Source: GitHub Education",
  ]) {
    assert.equal(hasText(renderer, label), true, `expected visible resource text: ${label}`);
  }

  unmount(renderer);
});
