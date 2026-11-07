require("ts-node").register({
  skipProject: true,
  transpileOnly: true,
  compilerOptions: {
    module: "CommonJS",
    moduleResolution: "node",
  },
});

const {
  TRANSFER_PLANNER_CANONICAL_COURSE_REGISTRY,
} = require("../../constants/transfer-planner-source");

function normalizeCourseCode(value) {
  const normalized = String(value ?? "").trim().toUpperCase().replace(/\s+/g, " ");
  const match = normalized.match(/^([A-Z][A-Z &]{0,14})\s+(\d{3}(?:\.\d+)?[A-Z]?)$/);
  if (!match) return normalized;
  return `${match[1].replace(/\s+/g, "")} ${match[2]}`;
}

const campuses = ["uw-seattle", "uw-bothell", "uw-tacoma"];
const codesBySchool = Object.fromEntries(
  campuses.map((schoolId) => [schoolId, new Set()])
);
for (const entry of TRANSFER_PLANNER_CANONICAL_COURSE_REGISTRY) {
  const codes = codesBySchool[entry.schoolId];
  if (!codes) continue;
  const code = normalizeCourseCode(entry.code);
  if (code) codes.add(code);
}
const output = Object.fromEntries(
  campuses.map((schoolId) => [
    schoolId,
    [...codesBySchool[schoolId]].sort((left, right) => left.localeCompare(right)),
  ])
);
process.stdout.write(JSON.stringify(output));
