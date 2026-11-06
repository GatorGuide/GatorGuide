import { localStorageService } from "@/services/storage/local-storage.service";
import {
  type Opportunity,
  normalizeOpportunity,
  normalizeOpportunityDate,
  OPPORTUNITY_STATUSES,
} from "@/constants/opportunities";
import { STARTER_OPPORTUNITIES } from "@/constants/starter-opportunities";
import { LOCAL_STORAGE_KEYS } from "@/services/storage/local-storage-contracts";
import { opportunityGatewayService } from "./opportunity-gateway.service";
import { fetchGitHubOpportunityCatalog } from "./opportunity-catalog-source";
export { fetchGitHubOpportunityCatalog, GITHUB_OPPORTUNITY_CATALOG_URL } from "./opportunity-catalog-source";

type OpportunityCatalogCachePayload = {
  schemaVersion: number;
  fetchedAt: string;
  opportunities: Opportunity[];
};

export type OpportunityCatalogResult = {
  opportunities: Opportunity[];
  source: "cache" | "github" | "bundled";
  fetchedAt: string | null;
};

const OPPORTUNITY_CATALOG_SCHEMA_VERSION = 2;

function sortOpportunities(opportunities: Opportunity[]) {
  return [...opportunities].sort((left, right) => {
    const leftDue = normalizeOpportunityDate(left.dueAt) ?? "9999-12-31T00:00:00.000Z";
    const rightDue = normalizeOpportunityDate(right.dueAt) ?? "9999-12-31T00:00:00.000Z";
    if (leftDue !== rightDue) return leftDue.localeCompare(rightDue);
    return left.title.localeCompare(right.title);
  });
}

function mergeOpportunityCatalogs(base: Opportunity[], incoming: Opportunity[]): Opportunity[] {
  const merged = new Map<string, Opportunity>();
  for (const opportunity of base ?? []) {
    const normalized = normalizeOpportunity(opportunity);
    if (normalized.opportunityId) merged.set(normalized.opportunityId, normalized);
  }
  for (const opportunity of incoming ?? []) {
    const normalized = normalizeOpportunity(opportunity);
    if (normalized.opportunityId) merged.set(normalized.opportunityId, normalized);
  }
  return sortOpportunities(Array.from(merged.values()));
}

class OpportunitiesService {
  private catalogStorageKey = LOCAL_STORAGE_KEYS.opportunitiesCatalog;
  private generatedStorageKey = LOCAL_STORAGE_KEYS.opportunitiesGenerated;
  private starterCatalog = sortOpportunities(
    STARTER_OPPORTUNITIES.map((opportunity) => normalizeOpportunity(opportunity))
  );

  private async readGeneratedCatalog(): Promise<Opportunity[]> {
    try {
      const raw = await localStorageService.getItem(this.generatedStorageKey);
      if (!raw) return [];
      const parsed = JSON.parse(raw) as unknown;
      if (!Array.isArray(parsed)) return [];
      return parsed
        .filter((item): item is Record<string, unknown> => !!item && typeof item === "object" && !Array.isArray(item))
        .map((item) => normalizeOpportunity(item as Partial<Opportunity>))
        .filter((opportunity) => !!opportunity.opportunityId);
    } catch {
      return [];
    }
  }

  private async withGeneratedOpportunities(opportunities: Opportunity[]) {
    // The checked-in catalog is authoritative for public records with the same ID.
    return mergeOpportunityCatalogs(await this.readGeneratedCatalog(), opportunities);
  }

  async readCatalogCache(): Promise<OpportunityCatalogCachePayload | null> {
    try {
      const raw = await localStorageService.getItem(this.catalogStorageKey);
      if (!raw) return null;
      const parsed = JSON.parse(raw) as OpportunityCatalogCachePayload;
      if (parsed.schemaVersion !== OPPORTUNITY_CATALOG_SCHEMA_VERSION || !Array.isArray(parsed.opportunities)) {
        return null;
      }
      const opportunities = parsed.opportunities
        .map((opportunity) => normalizeOpportunity(opportunity))
        .filter((opportunity) => !!opportunity.opportunityId);
      if (!opportunities.length) return null;
      return {
        schemaVersion: OPPORTUNITY_CATALOG_SCHEMA_VERSION,
        fetchedAt: normalizeOpportunityDate(parsed.fetchedAt) ?? new Date().toISOString(),
        opportunities,
      };
    } catch {
      return null;
    }
  }

  async writeCatalogCache(opportunities: Opportunity[]) {
    const payload: OpportunityCatalogCachePayload = {
      schemaVersion: OPPORTUNITY_CATALOG_SCHEMA_VERSION,
      fetchedAt: new Date().toISOString(),
      opportunities: mergeOpportunityCatalogs([], opportunities),
    };
    await localStorageService.setItem(this.catalogStorageKey, JSON.stringify(payload));
  }

  async loadCatalog(options: { preferCache?: boolean } = {}): Promise<OpportunityCatalogResult> {
    const preferCache = options.preferCache ?? true;
    const cached = await this.readCatalogCache();
    if (preferCache && cached) {
      return {
        opportunities: await this.withGeneratedOpportunities(cached.opportunities),
        source: "cache",
        fetchedAt: cached.fetchedAt,
      };
    }

    try {
      const opportunities = await fetchGitHubOpportunityCatalog();
      const fetchedAt = new Date().toISOString();
      await this.writeCatalogCache(opportunities);
      return {
        opportunities: await this.withGeneratedOpportunities(opportunities),
        source: "github",
        fetchedAt,
      };
    } catch {
      return {
        opportunities: await this.withGeneratedOpportunities(cached?.opportunities ?? this.starterCatalog),
        source: cached ? "cache" : "bundled",
        fetchedAt: cached?.fetchedAt ?? null,
      };
    }
  }

  async refreshCatalog(): Promise<OpportunityCatalogResult> {
    try {
      const opportunities = await fetchGitHubOpportunityCatalog();
      const fetchedAt = new Date().toISOString();
      await this.writeCatalogCache(opportunities);
      return {
        opportunities: await this.withGeneratedOpportunities(opportunities),
        source: "github",
        fetchedAt,
      };
    } catch {
      const cached = await this.readCatalogCache();
      return {
        opportunities: await this.withGeneratedOpportunities(cached?.opportunities ?? this.starterCatalog),
        source: cached ? "cache" : "bundled",
        fetchedAt: cached?.fetchedAt ?? null,
      };
    }
  }

  filterActiveCatalog(opportunities: Opportunity[]) {
    return sortOpportunities(
      (opportunities ?? []).filter((opportunity) => opportunity.status !== OPPORTUNITY_STATUSES.archived)
    );
  }

  findOpportunityById(opportunities: Opportunity[], opportunityId: string) {
    return (opportunities ?? []).find((opportunity) => opportunity.opportunityId === opportunityId);
  }

  findCollegeDeadlineOpportunity(opportunities: Opportunity[], collegeId: string | null | undefined) {
    const normalizedCollegeId = String(collegeId ?? "").trim();
    if (!normalizedCollegeId) return null;
    return (
      (opportunities ?? []).find(
        (opportunity) =>
          opportunity.type === "college_deadline" &&
          opportunity.college.collegeId === normalizedCollegeId
      ) ?? null
    );
  }

  async ensureCollegeDeadlineOpportunity(input: {
    collegeId?: string | null;
    collegeName?: string | null;
    collegeWebsite?: string | null;
  }) {
    const current = await this.loadCatalog({ preferCache: true });
    if (this.findCollegeDeadlineOpportunity(current.opportunities, input.collegeId)) return current;

    const response = await opportunityGatewayService.generateCollegeDeadlineOpportunity(input);
    if (!response.opportunity) return null;

    const generated = await this.readGeneratedCatalog();
    const nextGenerated = mergeOpportunityCatalogs(generated, [response.opportunity]);
    await localStorageService.setItem(this.generatedStorageKey, JSON.stringify(nextGenerated));
    return {
      ...current,
      opportunities: await this.withGeneratedOpportunities(current.opportunities),
      fetchedAt: new Date().toISOString(),
    };
  }
}

export const opportunitiesService = new OpportunitiesService();
