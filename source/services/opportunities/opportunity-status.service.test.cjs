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
const { normalizeUserOpportunityStatus } = require("@/constants/opportunities");
const { opportunityStatusService } = require("@/services/opportunities/opportunity-status.service");

test("guest opportunity progress is promoted locally and re-owned by the signed-in account", async () => {
  const accountStatus = normalizeUserOpportunityStatus(
    { progress: "saved", clientUpdatedAt: "2026-01-01T00:00:00.000Z" },
    "account-1",
    "opportunity-1"
  );
  const guestStatus = normalizeUserOpportunityStatus(
    { progress: "started", clientUpdatedAt: "2026-02-01T00:00:00.000Z" },
    "guest",
    "opportunity-1"
  );

  const result = await opportunityStatusService.mergeLocalStatuses(
    "account-1",
    { "opportunity-1": accountStatus },
    { promoteStatuses: { "opportunity-1": guestStatus } }
  );

  assert.equal(result.statuses["opportunity-1"].progress, "started");
  assert.equal(result.statuses["opportunity-1"].userId, "account-1");
  assert.deepEqual(Object.keys(result), ["statuses"]);
});
