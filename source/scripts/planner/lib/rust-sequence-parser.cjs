const fs = require("fs");
const path = require("path");
const { spawnSync } = require("child_process");

const REPOSITORY_ROOT = path.resolve(__dirname, "../../../..");
const AUTOMATION_ROOT = path.join(REPOSITORY_ROOT, "automation");
const MANIFEST_PATH = path.join(AUTOMATION_ROOT, "Cargo.toml");
let cargoRunAvailable = null;
let rustExecutable = null;

function findAutomationExecutable() {
  const configuredTargetRoot = process.env.CARGO_TARGET_DIR
    ? path.resolve(REPOSITORY_ROOT, process.env.CARGO_TARGET_DIR)
    : path.join(AUTOMATION_ROOT, "target");
  const executableName = process.platform === "win32"
    ? "gator-guide-automation.exe"
    : "gator-guide-automation";
  let targetEntries = [];
  try {
    targetEntries = fs.readdirSync(configuredTargetRoot, { withFileTypes: true });
  } catch {
    return null;
  }

  const profileDirectories = [path.join(configuredTargetRoot, "debug")];
  for (const entry of targetEntries) {
    if (entry.isDirectory() && entry.name !== "debug" && entry.name !== "release") {
      profileDirectories.push(path.join(configuredTargetRoot, entry.name, "debug"));
    }
  }
  for (const directory of profileDirectories) {
    const candidate = path.join(directory, executableName);
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }
  return null;
}

function runParserProcess(command, input) {
  const serializedInput = JSON.stringify(input);
  const commonOptions = {
    cwd: REPOSITORY_ROOT,
    encoding: "utf8",
    input: serializedInput,
    maxBuffer: 8 * 1024 * 1024,
    timeout: 30000,
    windowsHide: true,
  };

  let result;
  if (rustExecutable) {
    result = spawnSync(rustExecutable, [command], commonOptions);
  } else if (cargoRunAvailable !== false) {
    result = spawnSync(
      "cargo",
      [
        "run",
        "--quiet",
        "--offline",
        "--locked",
        "--manifest-path",
        MANIFEST_PATH,
        "--",
        command,
      ],
      commonOptions
    );
    if (result.status === 0) {
      cargoRunAvailable = true;
      rustExecutable = findAutomationExecutable();
    } else {
      cargoRunAvailable = false;
    }
  } else {
    return null;
  }

  if (result.error || result.status !== 0 || !result.stdout) {
    return null;
  }
  try {
    return JSON.parse(result.stdout);
  } catch {
    return null;
  }
}

function parseSequenceInlineSegments(texts) {
  if (!Array.isArray(texts) || !texts.length) {
    return [];
  }
  const segments = runParserProcess("parse-sequence-inline-segments", { texts });
  if (
    !Array.isArray(segments) ||
    segments.length !== texts.length ||
    segments.some((value) => !Array.isArray(value) || value.some((segment) => typeof segment !== "string"))
  ) {
    return null;
  }
  return segments;
}

function parseSequenceCourseCodes(rows) {
  if (!Array.isArray(rows) || !rows.length) {
    return [];
  }
  const courseCodes = runParserProcess("parse-sequence-course-codes", { rows });
  if (
    !Array.isArray(courseCodes) ||
    courseCodes.length !== rows.length ||
    courseCodes.some((value) => !Array.isArray(value) || value.some((code) => typeof code !== "string"))
  ) {
    return null;
  }
  return courseCodes;
}

module.exports = { parseSequenceCourseCodes, parseSequenceInlineSegments };
