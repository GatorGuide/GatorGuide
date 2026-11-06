"use strict";

/**
 * P20 refresh automation contract — validates deterministic workflow and publication boundaries.
 */

function validatePlannerRefreshWorkflow(doc = {}) {
  const errors = [];
  if (!doc.on?.schedule && !doc.on?.workflow_dispatch) {
    errors.push({ type: "missing-triggers" });
  }
  if (!doc.jobs || typeof doc.jobs !== "object") {
    errors.push({ type: "missing-jobs" });
  } else {
    const runTexts = Object.values(doc.jobs)
      .flatMap((job) => job.steps || [])
      .map((step) => String(step.run || step.uses || ""))
      .join("\n");
    if (!/npm ci|npm install/.test(runTexts)) {
      errors.push({ type: "missing-npm-ci" });
    }
    if (!/cargo run[\s\S]*refresh-planner|refreshPlannerData|planner:refresh/.test(runTexts)) {
      errors.push({ type: "missing-refresh-invocation" });
    }
    if (!/git push origin HEAD:main/.test(runTexts)) {
      errors.push({ type: "missing-data-publication" });
    }
    if (!/gh release create/.test(runTexts)) {
      errors.push({ type: "missing-automated-release" });
    }
  }
  if (doc.permissions?.contents !== "write") {
    errors.push({ type: "missing-write-permission" });
  }
  return { ok: errors.length === 0, errors };
}

function validateDormantCategoryWorkflow(doc = {}, category = "") {
  const errors = [];
  if (!doc.on?.schedule || !doc.on?.workflow_dispatch) {
    errors.push({ type: "missing-weekly-or-manual-trigger" });
  }
  const steps = Object.values(doc.jobs || {}).flatMap((job) => job.steps || []);
  const runTexts = steps.map((step) => String(step.run || step.uses || "")).join("\n");
  if (!/collect-opportunities/.test(runTexts) || !runTexts.includes(category)) {
    errors.push({ type: "missing-deterministic-category-collector" });
  }
  if (doc.permissions?.contents === "write" || /git push|gh release create/i.test(runTexts)) {
    errors.push({ type: "category-workflow-can-publish" });
  }
  if (/openai|gemini|anthropic|llm/i.test(runTexts)) {
    errors.push({ type: "ai-assisted-category-workflow" });
  }
  return { ok: errors.length === 0, errors };
}

function validateNoDiffSuccess({ beforeHash, afterHash, published }) {
  if (beforeHash === afterHash && published === true) {
    return { ok: false, reason: "no-diff-must-not-publish" };
  }
  if (beforeHash === afterHash && published === false) {
    return { ok: true, reason: "no-diff-success" };
  }
  return { ok: true, reason: "diff-present" };
}

module.exports = {
  validatePlannerRefreshWorkflow,
  validateDormantCategoryWorkflow,
  validateNoDiffSuccess,
};
