const path = require("node:path");
const { spawnSync } = require("node:child_process");

const sourceRoot = path.resolve(__dirname, "..", "..");
const manifestPath = path.resolve(sourceRoot, "..", "automation", "Cargo.toml");
const args = [
  "run",
  "--offline",
  "--locked",
  "--manifest-path",
  manifestPath,
  "--",
  "ingest-uw-catalog",
];
if (process.env.GATORGUIDE_PLANNER_CACHE_ONLY === "1") {
  args.push("--cache-only");
}

const result = spawnSync("cargo", args, {
  cwd: sourceRoot,
  stdio: "inherit",
});
if (result.error) {
  console.error(`Could not start Rust UW catalog ingestion: ${result.error.message}`);
  process.exitCode = 1;
} else {
  process.exitCode = result.status ?? 1;
}
