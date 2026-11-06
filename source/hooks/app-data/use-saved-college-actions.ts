import { useCallback, type Dispatch, type MutableRefObject, type SetStateAction } from "react";
import type { College } from "@/services/colleges/college.service";
import { savedCollegesService } from "@/services/colleges/saved-colleges.service";
import type { AppDataState } from "./app-data-state";

type UseSavedCollegeActionsArgs = {
  state: AppDataState;
  stateRef: MutableRefObject<AppDataState>;
  setState: Dispatch<SetStateAction<AppDataState>>;
};

export function useSavedCollegeActions({
  state,
  stateRef,
  setState,
}: UseSavedCollegeActionsArgs) {
  const addSavedCollege = useCallback(async (college: College) => {
    const mergedSavedColleges = savedCollegesService.mergeSavedCollegeLists(
      stateRef.current.savedColleges ?? [],
      [college]
    );
    setState((prev) => ({ ...prev, savedColleges: mergedSavedColleges }));
  }, [setState, stateRef]);

  const removeSavedCollege = useCallback(async (collegeId: string) => {
    setState((prev) => ({
      ...prev,
      savedColleges: (prev.savedColleges ?? []).filter((college) => String(college.id) !== String(collegeId)),
    }));
  }, [setState]);

  const isCollegeSaved = useCallback((collegeId: string) => {
    return (state.savedColleges ?? []).some((c) => String(c.id) === String(collegeId));
  }, [state.savedColleges]);

  return {
    addSavedCollege,
    removeSavedCollege,
    isCollegeSaved,
  };
}
