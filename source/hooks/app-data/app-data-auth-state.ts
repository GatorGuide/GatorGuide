import { DEFAULT_USER_STATE } from "@/constants/profile-defaults";
import type { AuthUser } from "@/services/auth/auth.service";
import { initialState, resolveUserState, type AppDataState } from "./app-data-state";

export function resolveAppDataForAccount(
  current: AppDataState,
  savedAccountState: AppDataState | null,
  authUser: AuthUser
): AppDataState {
  const currentIsSameAccount = current.user?.uid === authUser.uid && !current.user.isGuest;
  const base = currentIsSameAccount
    ? current
    : savedAccountState ?? (current.user?.isGuest ? current : initialState);
  const savedUser = base.user;

  return {
    ...base,
    user: {
      ...(savedUser ?? {}),
      uid: authUser.uid,
      name: authUser.name || savedUser?.name || authUser.email,
      email: authUser.email,
      isGuest: false,
      state: resolveUserState(savedUser?.state ?? DEFAULT_USER_STATE),
      major: savedUser?.major ?? "",
      gender: savedUser?.gender ?? "",
      gpa: savedUser?.gpa ?? "",
      resume: savedUser?.resume ?? "",
      transcript: savedUser?.transcript ?? "",
    },
  };
}
