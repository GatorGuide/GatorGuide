import { localStorageService } from "@/services/storage/local-storage.service";
import {
  type Opportunity,
  type OpportunityProgressState,
  type UserOpportunityStatus,
  isCompletedOpportunityProgress,
  normalizeUserOpportunityStatus,
  OPPORTUNITY_PROGRESS_STATES,
  resolveOpportunityCycleKey,
} from "@/constants/opportunities";
import {
  GUEST_OPPORTUNITY_USER_KEY,
  getOpportunityStatusesStorageKey,
  getOpportunityPendingStorageKey,
} from "@/services/storage/local-storage-contracts";

export type OpportunityStatusMap = Record<string, UserOpportunityStatus>;

const GUEST_USER_KEY = GUEST_OPPORTUNITY_USER_KEY;

function compareClientUpdatedAt(
  left: Pick<UserOpportunityStatus, "clientUpdatedAt"> | null | undefined,
  right: Pick<UserOpportunityStatus, "clientUpdatedAt"> | null | undefined
) {
  return String(left?.clientUpdatedAt ?? "").localeCompare(String(right?.clientUpdatedAt ?? ""));
}

class OpportunityStatusService {
  getGuestUserKey() {
    return GUEST_USER_KEY;
  }

  private getStatusesStorageKey(userKey: string) {
    return getOpportunityStatusesStorageKey(userKey);
  }

  async readLocalStatuses(userKey: string): Promise<OpportunityStatusMap> {
    try {
      const raw = await localStorageService.getItem(this.getStatusesStorageKey(userKey));
      if (!raw) return {};
      const parsed = JSON.parse(raw) as OpportunityStatusMap;
      return Object.fromEntries(
        Object.entries(parsed ?? {}).map(([opportunityId, status]) => [
          opportunityId,
          normalizeUserOpportunityStatus(status, String(status?.userId ?? userKey), opportunityId),
        ])
      );
    } catch {
      return {};
    }
  }

  async writeLocalStatuses(userKey: string, statuses: OpportunityStatusMap) {
    await localStorageService.setItem(this.getStatusesStorageKey(userKey), JSON.stringify(statuses));
  }

  async clearLocalStatuses(userKey: string) {
    await localStorageService.removeItem(this.getStatusesStorageKey(userKey));
  }

  /** Remove obsolete queued mutations from the pre-local-only implementation. */
  async clearPendingMutations(userKey: string) {
    await localStorageService.removeItem(getOpportunityPendingStorageKey(userKey));
  }

  mergeStatusMaps(primary: OpportunityStatusMap, secondary: OpportunityStatusMap): OpportunityStatusMap {
    const merged: OpportunityStatusMap = { ...primary };
    for (const [opportunityId, status] of Object.entries(secondary ?? {})) {
      const existing = merged[opportunityId];
      if (!existing || compareClientUpdatedAt(existing, status) <= 0) merged[opportunityId] = status;
    }
    return merged;
  }

  async mergeLocalStatuses(
    userId: string,
    localStatuses: OpportunityStatusMap,
    options: { promoteStatuses?: OpportunityStatusMap } = {}
  ) {
    const merged = this.mergeStatusMaps(localStatuses, options.promoteStatuses ?? {});
    const statuses = Object.fromEntries(
      Object.entries(merged).map(([opportunityId, status]) => [
        opportunityId,
        normalizeUserOpportunityStatus(
          { ...status, userId },
          userId,
          opportunityId
        ),
      ])
    );
    return { statuses };
  }

  buildProgressStatus(
    userId: string,
    opportunity: Opportunity,
    progress: OpportunityProgressState,
    existing?: UserOpportunityStatus | null
  ) {
    const nowIso = new Date().toISOString();
    const isDone = isCompletedOpportunityProgress(progress);
    return normalizeUserOpportunityStatus(
      {
        ...existing,
        progress,
        progressUpdatedAt: nowIso,
        isDone,
        doneAt: isDone ? nowIso : null,
        doneCycleKey: resolveOpportunityCycleKey(opportunity),
        clientUpdatedAt: nowIso,
      },
      userId,
      opportunity.opportunityId
    );
  }

  buildStatus(
    userId: string,
    opportunity: Opportunity,
    isDone: boolean,
    existing?: UserOpportunityStatus | null
  ) {
    return this.buildProgressStatus(
      userId,
      opportunity,
      isDone ? OPPORTUNITY_PROGRESS_STATES.submitted : OPPORTUNITY_PROGRESS_STATES.saved,
      existing
    );
  }
}

export const opportunityStatusService = new OpportunityStatusService();
