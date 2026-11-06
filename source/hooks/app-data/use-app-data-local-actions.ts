import { localStorageService } from "@/services/storage/local-storage.service";
import { STORAGE_KEYS } from "@/constants/schema";
import { getUserAppDataStorageKey } from "@/services/storage/local-storage-contracts";
import { useCallback, type Dispatch, type MutableRefObject, type SetStateAction } from "react";
import { normalizeQuestionnaireAnswers } from "@/services/app/questionnaire.enums";
import { migrateTransferPlannerLegacyCompletedCourses } from "@/services/planning/transfer-planner-cache.service";
import { errorLoggingService } from "@/services/logging/error-logging.service";
import { notificationsService } from "@/services/notifications/notifications.service";
import {
  createEmptyPlannerV2,
  type PlannerV2State,
} from "./planner-state-v2";
import {
  initialState,
  normalizeAppDataState,
  STORAGE_KEY,
  type AppDataState,
  type QuestionnaireAnswers,
} from "./app-data-state";

type UseAppDataLocalActionsArgs = {
  stateRef: MutableRefObject<AppDataState>;
  setState: Dispatch<SetStateAction<AppDataState>>;
  setRestoreVersion: Dispatch<SetStateAction<number>>;
};

export function useAppDataLocalActions({
  stateRef,
  setState,
  setRestoreVersion,
}: UseAppDataLocalActionsArgs) {
  const setQuestionnaireAnswers = useCallback(async (
    answers:
      | QuestionnaireAnswers
      | ((currentAnswers: QuestionnaireAnswers) => QuestionnaireAnswers)
  ) => {
    setState((prev) => {
      const nextAnswers =
        typeof answers === "function"
          ? answers(prev.questionnaireAnswers ?? {})
          : answers;
      const normalized = migrateTransferPlannerLegacyCompletedCourses(
        normalizeQuestionnaireAnswers(nextAnswers)
      );
      return { ...prev, questionnaireAnswers: { ...normalized } };
    });
  }, [setState]);

  const patchPlannerV2 = useCallback(async (patch: Partial<PlannerV2State>) => {
    setState((prev) => {
      const current = prev.plannerV2 ?? createEmptyPlannerV2();
      return {
        ...prev,
        plannerV2: {
          ...current,
          ...patch,
          reminderPreferences: {
            ...current.reminderPreferences,
            ...(patch.reminderPreferences || {}),
          },
          unavailableQuarters:
            patch.unavailableQuarters ?? current.unavailableQuarters,
          normalizedCourseIds:
            patch.normalizedCourseIds ?? current.normalizedCourseIds ?? [],
          overrides: patch.overrides ?? current.overrides,
        },
      };
    });
  }, [setState]);

  const restoreData = useCallback(async (data: AppDataState) => {
    setState(normalizeAppDataState(data));
    setRestoreVersion((current) => current + 1);
  }, [setRestoreVersion, setState]);

  const clearAll = useCallback(async () => {
    await notificationsService.clearManagedNotifications().catch(() => {});
    const user = stateRef.current.user;
    const storageKey = user && !user.isGuest ? getUserAppDataStorageKey(user.uid) : STORAGE_KEY;
    await localStorageService.multiRemove([storageKey, STORAGE_KEYS.appDataActiveUser]).catch((error) => {
      void errorLoggingService.captureException(error, {
        category: "storage",
        operation: "clear-app-data-primary",
        severity: "warn",
        handled: true,
        source: "use-app-data",
        metadata: {
          storageKey,
        },
      });
    });
    setState(initialState);
  }, [setState, stateRef]);

  return {
    setQuestionnaireAnswers,
    patchPlannerV2,
    restoreData,
    clearAll,
  };
}
