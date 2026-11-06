import {
  type Opportunity,
  normalizeOpportunity,
  normalizeOpportunityDate,
} from "@/constants/opportunities";
import { fetchJsonWithHandling } from "@/services/network/fetch-with-handling";

export const GITHUB_OPPORTUNITY_CATALOG_URL =
  "https://raw.githubusercontent.com/MarsLuay/GatorGuide/main/source/data/starter-opportunities.json";

function sortOpportunities(opportunities: Opportunity[]) {
  return [...opportunities].sort((left, right) => {
    const leftDue = normalizeOpportunityDate(left.dueAt) ?? "9999-12-31T00:00:00.000Z";
    const rightDue = normalizeOpportunityDate(right.dueAt) ?? "9999-12-31T00:00:00.000Z";
    if (leftDue !== rightDue) return leftDue.localeCompare(rightDue);
    return left.title.localeCompare(right.title);
  });
}

export async function fetchGitHubOpportunityCatalog(fetchImpl?: typeof fetch): Promise<Opportunity[]> {
  const payload = await fetchJsonWithHandling<unknown>(GITHUB_OPPORTUNITY_CATALOG_URL, {
    fetchImpl,
    operation: "load GitHub opportunity catalog",
    retries: 2,
    timeoutMs: 15000,
    invalidJsonMessage: "GitHub opportunity catalog returned invalid JSON.",
  });
  if (!Array.isArray(payload)) throw new Error("GitHub opportunity catalog must be a JSON array.");

  const opportunities = payload
    .filter((item): item is Record<string, unknown> => !!item && typeof item === "object" && !Array.isArray(item))
    .map((item) => normalizeOpportunity(item as Partial<Opportunity>))
    .filter((opportunity) => !!opportunity.opportunityId);
  if (!opportunities.length) throw new Error("GitHub opportunity catalog contains no valid opportunities.");
  return sortOpportunities(opportunities);
}
