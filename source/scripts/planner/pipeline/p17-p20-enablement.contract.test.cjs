"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const {
  REQUIRED_LOCALE_COUNT,
  validateTranslationPair,
  validateLocaleSet,
} = require("./i18n/locale-contract.cjs");
const {
  createMemoryPlannerStateRepository,
  projectTimelineOffline,
} = require("./offline/local-first.cjs");
const {
  assertReducedMotionHonored,
  assertTouchTargetMin,
  assertTextScaleSurvives,
} = require("./a11y/a11y-contract.cjs");
const {
  validatePlannerRefreshWorkflow,
  validateDormantCategoryWorkflow,
  validateNoDiffSuccess,
} = require("./automation/refresh-workflow-contract.cjs");

const ROOT = path.join(__dirname, "../../..");

test("P17 placeholder multiset + 16-locale set", () => {
  const pair = validateTranslationPair(
    "Due {date} for {{course}}",
    "Échéance {date} pour {{course}}",
    { glossary: { MATH: true } }
  );
  assert.equal(pair.ok, true);
  const bad = validateTranslationPair("Hello {name}", "Hola");
  assert.equal(bad.ok, false);
  const locales = Array.from({ length: REQUIRED_LOCALE_COUNT }, (_, i) => `l${i}`);
  assert.equal(validateLocaleSet(locales).ok, true);
  assert.equal(validateLocaleSet(locales.slice(0, 8)).ok, false);
});

test("P18 local-first repo offline write/restart + remote revision rule", async () => {
  const repo = createMemoryPlannerStateRepository({
    activeTarget: { campus: "seattle" },
    revision: 1,
  });
  await repo.write({ activeTarget: { campus: "bothell" } });
  const restarted = createMemoryPlannerStateRepository(await repo.read());
  const again = await restarted.read();
  assert.equal(again.activeTarget.campus, "bothell");

  const conflict = await repo.mergeRemote({
    activeTarget: { campus: "tacoma" },
    revision: 99,
  });
  assert.equal(conflict.winner, "remote");
  assert.equal(conflict.state.activeTarget.campus, "tacoma");

  const timeline = projectTimelineOffline({
    plan: { quarters: [{ key: "2027-fall" }] },
    opportunities: [{ id: "1", dueAt: "2026-12-01" }],
  });
  assert.equal(timeline[0].id, "opp:1");
});

test("P18 TS port + calendar offline product path exist", () => {
  const tsPort = path.join(ROOT, "services/planning/planner-local-repository.ts");
  const appBridge = path.join(ROOT, "hooks/app-data/planner-offline.ts");
  const calendarHelper = path.join(
    ROOT,
    "components/pages/deadline-calendar/calendar-offline-timeline.ts"
  );
  const controller = path.join(
    ROOT,
    "components/pages/deadline-calendar/useDeadlineCalendarController.ts"
  );

  assert.ok(fs.existsSync(tsPort));
  assert.ok(fs.existsSync(appBridge));
  assert.ok(fs.existsSync(calendarHelper));

  const tsText = fs.readFileSync(tsPort, "utf8");
  assert.match(tsText, /export function createMemoryPlannerStateRepository/);
  assert.match(tsText, /export function projectTimelineOffline/);
  assert.match(tsText, /local state is the source of truth/i);

  const bridgeText = fs.readFileSync(appBridge, "utf8");
  assert.match(bridgeText, /projectOfflineTimelineFromPlannerV2/);
  assert.match(bridgeText, /projectTimelineOffline/);
  assert.match(bridgeText, /Local AppData is the source of truth/i);

  const helperText = fs.readFileSync(calendarHelper, "utf8");
  assert.match(helperText, /projectPlannerV2OfflineCalendarEntries/);
  assert.match(helperText, /projectOfflineTimelineFromPlannerV2/);

  const controllerText = fs.readFileSync(controller, "utf8");
  assert.match(controllerText, /projectPlannerV2OfflineCalendarEntries/);
});

test("P19 a11y reduced-motion, touch target, text scale", () => {
  assert.equal(
    assertReducedMotionHonored({
      "prefers-reduced-motion": "reduce",
      cssText: "animation: none; transition: none;",
    }).ok,
    true
  );
  assert.equal(assertTouchTargetMin(44).ok, true);
  assert.equal(assertTouchTargetMin(24).ok, false);
  assert.equal(
    assertTextScaleSurvives({
      actionsAt100: ["save", "lock"],
      actionsAtScale: ["save", "lock"],
    }).ok,
    true
  );
});

test("P20 Rust planner refresh publishes data and creates a release", () => {
  const workflow = {
    on: { schedule: [{ cron: "0 12 * * 1" }], workflow_dispatch: {} },
    permissions: { contents: "write" },
    jobs: {
      refresh: {
        steps: [
          { run: "npm ci" },
          { run: "cargo run --locked --manifest-path automation/Cargo.toml -- refresh-planner" },
          { run: "git add source/data/starter-opportunities.json && git push origin HEAD:main" },
          { run: "gh release create planner-refresh-${GITHUB_RUN_ID}" },
        ],
      },
    },
  };
  assert.equal(validatePlannerRefreshWorkflow(workflow).ok, true);
  assert.equal(
    validateNoDiffSuccess({ beforeHash: "a", afterHash: "a", published: false }).ok,
    true
  );
  assert.equal(
    validateNoDiffSuccess({ beforeHash: "a", afterHash: "a", published: true }).ok,
    false
  );
});

test("P20 scholarship and internship collectors are deterministic and non-publishing", () => {
  for (const category of ["scholarships", "internships"]) {
    const result = validateDormantCategoryWorkflow(
      {
        on: { schedule: [{ cron: "0 13 * * 1" }], workflow_dispatch: {} },
        permissions: { contents: "read" },
        jobs: {
          review: {
            steps: [
              { run: `cargo run --locked --manifest-path automation/Cargo.toml -- collect-opportunities --category ${category}` },
            ],
          },
        },
      },
      category
    );
    assert.equal(result.ok, true, JSON.stringify(result.errors));
  }
});

test("P20 repository keeps only the planner workflow active and two dormant category workflows", () => {
  const repositoryRoot = path.resolve(ROOT, "..");
  const workflowDir = path.join(repositoryRoot, ".github", "workflows");
  assert.deepEqual(fs.readdirSync(workflowDir).sort(), [
    "internship-refresh.yml",
    "planner-refresh.yml",
    "scholarship-refresh.yml",
  ]);
  assert.equal(fs.existsSync(path.join(repositoryRoot, ".github", "dependabot.yml")), false);

  const planner = fs.readFileSync(path.join(workflowDir, "planner-refresh.yml"), "utf8");
  assert.match(planner, /cron: "0 12 \* \* 1"/);
  assert.match(planner, /cargo run[\s\S]*refresh-planner/);
  assert.match(planner, /git push origin HEAD:main/);
  assert.match(planner, /gh release create/);
  assert.match(planner, /actions\/deploy-pages@v4/);

  for (const [file, category] of [
    ["scholarship-refresh.yml", "scholarships"],
    ["internship-refresh.yml", "internships"],
  ]) {
    const workflow = fs.readFileSync(path.join(workflowDir, file), "utf8");
    assert.match(workflow, /cron:/);
    assert.match(workflow, /workflow_dispatch:/);
    assert.match(workflow, new RegExp(`collect-opportunities[\\s\\S]*--category ${category}`));
    assert.match(workflow, /contents: read/);
    assert.match(workflow, /actions\/upload-artifact@v4/);
    assert.doesNotMatch(workflow, /git push|gh release create|openai|gemini|anthropic|llm/i);
  }
});
