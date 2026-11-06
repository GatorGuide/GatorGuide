import { localStorageService } from "@/services/storage/local-storage.service";
import { useEffect, type Dispatch, type SetStateAction } from "react";
import { STORAGE_KEYS } from "@/constants/schema";
import { getUserAppDataStorageKey } from "@/services/storage/local-storage-contracts";
import { errorLoggingService } from "@/services/logging/error-logging.service";
import { STORAGE_KEY, type AppDataState } from "./app-data-state";
import { parsePersistedAppDataState, serializeAppDataState } from "./app-data-persistence";

type UseAppDataStorageArgs = {
  isHydrated: boolean;
  setIsHydrated: Dispatch<SetStateAction<boolean>>;
  state: AppDataState;
  setState: Dispatch<SetStateAction<AppDataState>>;
};

export function useAppDataStorage({ isHydrated, setIsHydrated, state, setState }: UseAppDataStorageArgs) {
  useEffect(() => {
    let mounted = true;
    (async () => {
      try {
        const activeUserId = await localStorageService.getItem(STORAGE_KEYS.appDataActiveUser);
        const accountStorageKey = activeUserId ? getUserAppDataStorageKey(activeUserId) : null;
        const raw =
          (accountStorageKey ? await localStorageService.getItem(accountStorageKey) : null) ??
          (await localStorageService.getItem(STORAGE_KEY));
        if (!mounted) return;
        if (raw) setState(parsePersistedAppDataState(raw).state);
      } catch (error) {
        void errorLoggingService.captureException(error, {
          category: "storage",
          operation: "hydrate-app-data",
          severity: "warn",
          handled: true,
          source: "use-app-data",
          metadata: { storageKey: STORAGE_KEY },
        });
      } finally {
        if (mounted) setIsHydrated(true);
      }
    })();
    return () => {
      mounted = false;
    };
  }, [setIsHydrated, setState]);

  useEffect(() => {
    if (!isHydrated) return;
    const userId = state.user && !state.user.isGuest ? state.user.uid : null;
    const storageKey = userId ? getUserAppDataStorageKey(userId) : STORAGE_KEY;

    void (async () => {
      try {
        await localStorageService.setItem(storageKey, serializeAppDataState(state));
        if (userId) {
          await localStorageService.setItem(STORAGE_KEYS.appDataActiveUser, userId);
          await localStorageService.removeItem(STORAGE_KEY);
        } else {
          await localStorageService.removeItem(STORAGE_KEYS.appDataActiveUser);
        }
      } catch (error) {
        void errorLoggingService.captureException(error, {
          category: "storage",
          operation: "persist-app-data-locally",
          severity: "warn",
          handled: true,
          source: "use-app-data",
          metadata: { storageKey, hasUser: !!state.user },
        });
      }
    })();
  }, [isHydrated, state]);
}
