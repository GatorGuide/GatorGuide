import { useCallback, type Dispatch, type SetStateAction } from "react";
import type { AppDataState, User } from "./app-data-state";

type UseAppDataUserActionsArgs = {
  setState: Dispatch<SetStateAction<AppDataState>>;
};

export function useAppDataUserActions({ setState }: UseAppDataUserActionsArgs) {
  const updateUser = useCallback(async (patch: Partial<User>) => {
    setState((prev) => {
      if (!prev.user) return prev;
      return {
        ...prev,
        user: { ...prev.user, ...patch },
      };
    });
  }, [setState]);

  const setOnboardingSeen = useCallback(async (seen: boolean) => {
    setState((prev) => {
      if (!prev.user) return prev;
      return {
        ...prev,
        user: { ...prev.user, hasSeenOnboarding: seen },
      };
    });
  }, [setState]);

  return {
    patchUserLocally: updateUser,
    updateUser,
    setOnboardingSeen,
  };
}
