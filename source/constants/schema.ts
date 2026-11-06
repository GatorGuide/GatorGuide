type ValueOf<T> = T[keyof T];

export const APP_DATA_SCHEMA_VERSION = 1 as const;

export const STORAGE_KEYS = {
  appData: "gatorguide:appdata:v1",
  appDataUserPrefix: "gatorguide:appdata:user:",
  appDataActiveUser: "gatorguide:appdata:active-user:v1",
  hasSeenStartup: "gatorguide:hasSeenStartup",
  pendingAccountData: "gatorguide:pending-account-data",
  guestProfileShow: "gatorguide:guestProfile:show",
  guestRoadmapShow: "gatorguide:guestRoadmap:show",
  pendingDeleteAccount: "gatorguide:pending-delete-account",
  onboardingDebugLog: "gatorguide:onboarding-debug-log:v1",
  notificationsManaged: "gatorguide:notifications:managed:v1",
  errorLogQueue: "gatorguide:error-logs:queue:v1",
  opportunitiesCatalog: "gatorguide:opportunities:catalog:v2",
  opportunitiesGenerated: "gatorguide:opportunities:generated:v1",
  opportunitiesGuestStatuses: "gatorguide:opportunities:statuses:guest:v1",
  opportunitiesGuestPending: "gatorguide:opportunities:pending:guest:v1",
  opportunitiesStatusesPrefix: "gatorguide:opportunities:statuses:",
  opportunitiesPendingPrefix: "gatorguide:opportunities:pending:",
  roadmapDataPrefix: "gatorguide:roadmap:data:",
} as const;

export const LOCAL_DOCUMENTS_DIR_NAME = "gatorguide_docs" as const;
export const FIREBASE_STORAGE_ROOT = "users" as const;

export const PROFILE_FIELD_KEYS = {
  uid: "uid",
  name: "name",
  email: "email",
  isGuest: "isGuest",
  avatar: "avatar",
  state: "state",
  major: "major",
  gender: "gender",
  residencyType: "residencyType",
  gpa: "gpa",
  resume: "resume",
  transcript: "transcript",
  isProfileComplete: "isProfileComplete",
  hasSeenOnboarding: "hasSeenOnboarding",
} as const;

export type ProfileFieldKey = ValueOf<typeof PROFILE_FIELD_KEYS>;

export const PROFILE_DOCUMENT_TYPES = {
  resume: "resume",
  transcript: "transcript",
  avatar: "avatar",
} as const;

export type ProfileDocumentType = ValueOf<typeof PROFILE_DOCUMENT_TYPES>;

export const QUESTIONNAIRE_SECTION_IDS = {
  data: "sectionData",
  academicPlan: "sectionAcademicPlan",
  personalStatement: "sectionPersonalStatement",
  occupation: "sectionOccupation",
} as const;

export type QuestionnaireSectionId = ValueOf<typeof QUESTIONNAIRE_SECTION_IDS>;

export const QUESTIONNAIRE_FIELD_IDS = {
  advisor: "advisor",
  gpa: "gpa",
  lgbtqCommunity: "lgbtqCommunity",
  weather: "weather",
  costOfAttendance: "costOfAttendance",
  graduationRate: "graduationRate",
  acceptanceRate: "acceptanceRate",
  location: "location",
  collegeVibe: "collegeVibe",
  transportation: "transportation",
  companiesNearby: "companiesNearby",
  inStateOutOfState: "inStateOutOfState",
  housing: "housing",
  studentStaffRatio: "studentStaffRatio",
  internationalStudentRatio: "internationalStudentRatio",
  taxRates: "taxRates",
  ranking: "ranking",
  researchOpportunities: "researchOpportunities",
  major: "major",
  continueEducation: "continueEducation",
  graduationDate: "graduationDate",
  quarterSemesterSystem: "quarterSemesterSystem",
  transferStudentRate: "transferStudentRate",
  extracurriculars: "extracurriculars",
  timeZone: "timeZone",
  deadline: "deadline",
  majorExploration: "majorExploration",
  certifications: "certifications",
  associatesForTransfer: "associatesForTransfer",
  salary: "salary",
  workEnvironment: "workEnvironment",
  yearsToComplete: "yearsToComplete",
  demand: "demand",
  howCompetitive: "howCompetitive",
  personalInterest: "personalInterest",
  typesOfOccupation: "typesOfOccupation",
  opportunitiesToExpatriate: "opportunitiesToExpatriate",
  requiredCourses: "requiredCourses",
  recommendedCourses: "recommendedCourses",
  resourcesOnCampus: "resourcesOnCampus",
  personalStatementFocus: "personalStatementFocus",
  internships: "internships",
  entryLevelPosition: "entryLevelPosition",
  resumeSkills: "resumeSkills",
  platforms: "platforms",
  classSize: "classSize",
  collegeSetting: "collegeSetting",
  environment: "environment",
  useWeightedSearch: "useWeightedSearch",
} as const;

export type QuestionnaireFieldId = ValueOf<typeof QUESTIONNAIRE_FIELD_IDS>;

export const PROFILE_QUESTIONNAIRE_FIELD_IDS = [
  QUESTIONNAIRE_FIELD_IDS.costOfAttendance,
  QUESTIONNAIRE_FIELD_IDS.classSize,
  QUESTIONNAIRE_FIELD_IDS.transportation,
  QUESTIONNAIRE_FIELD_IDS.companiesNearby,
  QUESTIONNAIRE_FIELD_IDS.inStateOutOfState,
  QUESTIONNAIRE_FIELD_IDS.housing,
  QUESTIONNAIRE_FIELD_IDS.ranking,
  QUESTIONNAIRE_FIELD_IDS.continueEducation,
  QUESTIONNAIRE_FIELD_IDS.extracurriculars,
] as const;

export type ProfileQuestionnaireFieldId =
  (typeof PROFILE_QUESTIONNAIRE_FIELD_IDS)[number];

export function getOpportunityStatusesStorageKey(userKey: string) {
  return `${STORAGE_KEYS.opportunitiesStatusesPrefix}${userKey}`;
}

export function getOpportunityPendingStorageKey(userKey: string) {
  return `${STORAGE_KEYS.opportunitiesPendingPrefix}${userKey}`;
}

export function getResumeStorageKey(uid: string) {
  return `${PROFILE_DOCUMENT_TYPES.resume}:${uid}`;
}

export function getTranscriptStorageKey(uid: string) {
  return `${PROFILE_DOCUMENT_TYPES.transcript}:${uid}`;
}

export function getAvatarStorageKey(uid: string) {
  return `${PROFILE_DOCUMENT_TYPES.avatar}:${uid}`;
}

export function getRoadmapDocumentStorageKey(userId: string, docType: string) {
  return `roadmap:${userId}:${docType}`;
}

export function buildFirebaseUserStoragePath(
  uid: string,
  ...segments: string[]
) {
  return [FIREBASE_STORAGE_ROOT, uid, ...segments].join("/");
}

export function buildLocalDocumentSubdirectory(
  type: "resume" | "transcript" | "avatar" | "roadmap",
  userId: string
) {
  return `${type}_${userId}`;
}
