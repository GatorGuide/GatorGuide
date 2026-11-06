import { localStorageService } from "@/services/storage/local-storage.service";
import { useCallback, type Dispatch, type MutableRefObject, type SetStateAction } from "react";
import { STORAGE_KEYS } from "@/constants/schema";
import {
  getAvatarStorageKey,
  getOpportunityPendingStorageKey,
  getOpportunityStatusesStorageKey,
  getRoadmapDocumentStorageKey,
  getResumeStorageKey,
  getTranscriptStorageKey,
  getUserAppDataStorageKey,
  getUserRoadmapStorageKey,
} from "@/services/storage/local-storage-contracts";
import { authService, type AuthUser } from "@/services/auth/auth.service";
import { resolveAppDataForAccount } from "./app-data-auth-state";
import { errorLoggingService } from "@/services/logging/error-logging.service";
import { notificationsService } from "@/services/notifications/notifications.service";
import { storageService } from "@/services/storage/storage.service";
import {
  initialState,
  resolveUserState,
  STORAGE_KEY,
  type AppDataState,
  type User,
} from "./app-data-state";
import { parsePersistedAppDataState, serializeAppDataState } from "./app-data-persistence";

type UseAppDataAuthActionsArgs = {
  stateRef: MutableRefObject<AppDataState>;
  setState: Dispatch<SetStateAction<AppDataState>>;
};

async function readAccountState(uid: string): Promise<AppDataState | null> {
  try {
    const raw = await localStorageService.getItem(getUserAppDataStorageKey(uid));
    if (!raw) return null;
    const state = parsePersistedAppDataState(raw).state;
    return state.user?.uid === uid && !state.user.isGuest ? state : null;
  } catch {
    return null;
  }
}

export function useAppDataAuthActions({ stateRef, setState }: UseAppDataAuthActionsArgs) {
  const activateAccount = useCallback(async (authUser: AuthUser) => {
    const current = stateRef.current;
    const currentIsSameAccount = current.user?.uid === authUser.uid && !current.user.isGuest;
    const savedAccountState = currentIsSameAccount ? current : await readAccountState(authUser.uid);
    setState(resolveAppDataForAccount(current, savedAccountState, authUser));
  }, [setState, stateRef]);

  const signIn = useCallback(async (
    input: Pick<User, "name" | "email"> & { password: string; isSignUp: boolean }
  ) => {
    const authUser = await authService.signIn({
      name: input.name,
      email: input.email,
      password: input.password,
      isSignUp: input.isSignUp,
    });
    await activateAccount(authUser);
  }, [activateAccount]);

  const signInWithAuthUser = useCallback(async (authUser: AuthUser) => {
    await activateAccount(authUser);
  }, [activateAccount]);

  const signInAsGuest = useCallback(async () => {
    const current = stateRef.current;
    const base = current.user?.isGuest ? current : initialState;
    const uid = current.user?.isGuest ? current.user.uid : `guest-${Date.now()}`;
    setState({
      ...base,
      user: {
        uid,
        name: "Guest User",
        email: `${uid}@gatorguide.local`,
        isGuest: true,
        state: resolveUserState(base.user?.state),
        major: base.user?.major ?? "",
        gender: base.user?.gender ?? "",
        gpa: base.user?.gpa ?? "",
        resume: base.user?.resume ?? "",
        transcript: base.user?.transcript ?? "",
      },
    });
  }, [setState, stateRef]);

  const signOut = useCallback(async () => {
    const current = stateRef.current;
    if (current.user && !current.user.isGuest) {
      try {
        await localStorageService.setItem(
          getUserAppDataStorageKey(current.user.uid),
          serializeAppDataState(current)
        );
      } catch (error) {
        void errorLoggingService.captureException(error, {
          category: "storage",
          operation: "persist-account-data-before-signout",
          severity: "warn",
          handled: true,
          source: "use-app-data",
        });
      }
    }

    try {
      await authService.signOut();
    } catch (error) {
      void errorLoggingService.captureException(error, {
        category: "auth",
        operation: "sign-out",
        severity: "warn",
        handled: true,
        source: "use-app-data",
      });
    }
    await notificationsService.clearManagedNotifications().catch(() => {});
    setState(initialState);
    await localStorageService.multiRemove([STORAGE_KEYS.appDataActiveUser, STORAGE_KEY]);
  }, [setState, stateRef]);

  const deleteAccount = useCallback(async () => {
    const user = stateRef.current.user;
    const uid = user && !user.isGuest ? user.uid : null;
    await authService.deleteAccount();
    await notificationsService.clearManagedNotifications().catch(() => {});
    if (uid) {
      await storageService.deleteAllUserLocalDocuments(uid).catch(() => {});
      const knownKeys = [
        getUserAppDataStorageKey(uid),
        getUserRoadmapStorageKey(uid),
        getOpportunityStatusesStorageKey(uid),
        getOpportunityPendingStorageKey(uid),
        getResumeStorageKey(uid),
        getTranscriptStorageKey(uid),
        getAvatarStorageKey(uid),
        ...["resume", "transcripts", "personalStatement", "recommendation1", "recommendation2"].map(
          (docType) => getRoadmapDocumentStorageKey(uid, docType)
        ),
      ];
      const keys = await localStorageService.getAllKeys().catch(() => []);
      const roadmapDocumentPrefix = `roadmap:${uid}:`;
      await localStorageService.multiRemove([
        ...knownKeys,
        ...keys.filter((key) => key.startsWith(roadmapDocumentPrefix)),
      ]).catch(() => {});
    }
    setState(initialState);
    await localStorageService.multiRemove([
      STORAGE_KEY,
      STORAGE_KEYS.appDataActiveUser,
      STORAGE_KEYS.guestProfileShow,
    ]).catch(() => {});
  }, [setState, stateRef]);

  return {
    signIn,
    signInWithAuthUser,
    signInAsGuest,
    signOut,
    deleteAccount,
  };
}
