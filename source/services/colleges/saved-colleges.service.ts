import type { College } from "./college.service";
import { normalizeRateValue } from "@/utils/locale-format";

const uniqueStrings = (values: string[] | undefined | null) =>
  Array.from(new Set((values ?? []).map((value) => String(value ?? "").trim()).filter(Boolean)));

const pickDefined = <T>(primary: T | null | undefined, fallback: T | null | undefined): T | null | undefined =>
  primary !== undefined && primary !== null && primary !== ("" as unknown as T) ? primary : fallback;

function mergeCollegeSnapshot(base: College, incoming: College): College {
  return {
    ...base,
    ...incoming,
    location: {
      city: String(pickDefined(incoming.location?.city, base.location?.city) ?? "").trim(),
      state: String(pickDefined(incoming.location?.state, base.location?.state) ?? "").trim(),
    },
    tuition: pickDefined(incoming.tuition, base.tuition) ?? null,
    tuitionInState: pickDefined(incoming.tuitionInState, base.tuitionInState) ?? null,
    tuitionOutOfState: pickDefined(incoming.tuitionOutOfState, base.tuitionOutOfState) ?? null,
    studentSize: pickDefined(incoming.studentSize, base.studentSize) ?? null,
    admissionRate: normalizeRateValue(pickDefined(incoming.admissionRate, base.admissionRate) ?? null),
    completionRate: normalizeRateValue(pickDefined(incoming.completionRate, base.completionRate) ?? null),
    website: pickDefined(incoming.website, base.website) ?? null,
    priceCalculator: pickDefined(incoming.priceCalculator, base.priceCalculator) ?? null,
    programs: uniqueStrings([...(base.programs ?? []), ...(incoming.programs ?? [])]),
    matchScore: pickDefined(incoming.matchScore, base.matchScore) ?? undefined,
    degreesAwarded: incoming.degreesAwarded ?? base.degreesAwarded ?? null,
    locale: pickDefined(incoming.locale, base.locale) ?? null,
    avgNetPriceOverall: pickDefined(incoming.avgNetPriceOverall, base.avgNetPriceOverall) ?? null,
    attendanceAcademicYear: pickDefined(incoming.attendanceAcademicYear, base.attendanceAcademicYear) ?? null,
    pellGrantRate: normalizeRateValue(pickDefined(incoming.pellGrantRate, base.pellGrantRate) ?? null),
    medianDebtCompletersOverall:
      pickDefined(incoming.medianDebtCompletersOverall, base.medianDebtCompletersOverall) ?? null,
    raw: incoming.raw ?? base.raw,
  };
}

class SavedCollegesService {
  mergeSavedCollegeLists(primary: College[], secondary: College[]): College[] {
    const merged = new Map<string, College>();
    for (const college of primary ?? []) merged.set(String(college.id), college);
    for (const college of secondary ?? []) {
      const id = String(college.id);
      const existing = merged.get(id);
      merged.set(id, existing ? mergeCollegeSnapshot(existing, college) : college);
    }
    return Array.from(merged.values());
  }
}

export const savedCollegesService = new SavedCollegesService();
