const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const yaml = require("yaml");

const SOURCE_ROOT = path.resolve(__dirname, "../..");
const REPO_ROOT = path.resolve(SOURCE_ROOT, "..");
const workflowPath = path.join(
  REPO_ROOT,
  ".github",
  "workflows",
  "transfer-category-equivalencies-monthly.yml"
);
const workflowText = fs.readFileSync(workflowPath, "utf8");
const workflow = yaml.parse(workflowText);
const allSteps = Object.values(workflow.jobs).flatMap((job) => job.steps ?? []);
const allRunText = allSteps.map((step) => String(step.run ?? "")).join("\n");

test("monthly workflow refreshes and publishes only UW-GRC equivalency outputs", () => {
  assert.deepEqual(
    workflow.on.schedule.map((schedule) => schedule.cron),
    ["0 12 1 * *"]
  );
  assert.ok(Object.hasOwn(workflow.on, "workflow_dispatch"));
  assert.equal(workflow.permissions.contents, "write");
  assert.equal(workflow.concurrency.group, "gator-guide-refresh-and-pages");
  assert.equal(workflow.jobs["refresh-transfer-equivalencies"].if, "github.ref == 'refs/heads/main'");
  assert.match(allRunText, /parse-transfer-planner-equivalency-guide\.cjs/);
  assert.match(allRunText, /automation\/equivalency-refresh\/Cargo\.toml/);
  assert.match(
    allRunText,
    /source\/constants\/transfer-planner-source\/equivalency-guide\.generated\.ts/
  );
  assert.match(allRunText, /source\/constants\/transfer-equivalency-catalog\.generated\.ts/);
  assert.match(allRunText, /git push origin HEAD:main/);
  assert.match(allRunText, /git diff --cached --quiet/);
  assert.doesNotMatch(allRunText, /source\/constants\/locales\/en\.json/);
});

test("guide parser regenerates the catalog from rules and keeps the date placeholder blank", () => {
  const parser = fs.readFileSync(
    path.join(__dirname, "parse-transfer-planner-equivalency-guide.cjs"),
    "utf8"
  );
  assert.match(parser, /render-transfer-equivalency-catalog-typescript/);
  assert.match(parser, /equivalency-refresh/);
  assert.match(parser, /transfer-equivalency-catalog\.generated\.ts/);

  const description =
    require("../../constants/locales/en.json").resources
      .transferCategoryEquivalenciesDescription;
  assert.match(description, /Double check using Equivalency Guide\./);
  assert.ok(
    description.endsWith("\nDouble check using Equivalency Guide. Last updated at ____.")
  );
});
