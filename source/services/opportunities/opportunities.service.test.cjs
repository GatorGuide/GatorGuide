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
const { STARTER_OPPORTUNITIES } = require("@/constants/starter-opportunities");
const {
  fetchGitHubOpportunityCatalog,
  GITHUB_OPPORTUNITY_CATALOG_URL,
} = require("@/services/opportunities/opportunity-catalog-source");

test("shared opportunity catalog is fetched as public GitHub JSON and normalized", async () => {
  let requestedUrl = null;
  const fetchImpl = async (url) => {
    requestedUrl = String(url);
    return new Response(JSON.stringify(STARTER_OPPORTUNITIES), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  const opportunities = await fetchGitHubOpportunityCatalog(fetchImpl);

  assert.equal(requestedUrl, GITHUB_OPPORTUNITY_CATALOG_URL);
  assert.match(requestedUrl, /^https:\/\/raw\.githubusercontent\.com\/MarsLuay\/GatorGuide\/main\//);
  assert.ok(opportunities.length > 0);
  assert.ok(opportunities.every((opportunity) => opportunity.opportunityId));
});

test("shared opportunity catalog rejects invalid non-array payloads", async () => {
  const fetchImpl = async () =>
    new Response(JSON.stringify({ opportunities: STARTER_OPPORTUNITIES }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });

  await assert.rejects(
    fetchGitHubOpportunityCatalog(fetchImpl),
    /must be a JSON array/
  );
});
