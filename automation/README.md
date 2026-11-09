# Deterministic scheduled automations

A read-only Windows Task Scheduler query found no matching GatorGuide/planner task. The repository has an active weekly planner workflow and a dedicated monthly UW Green River transfer-equivalency refresh; scholarship and internship review workflows remain disabled.

GitHub may still list platform-managed dynamic workflows in Actions. Dependabot update configuration was removed, Dependabot security auto-updates were disabled, and CodeQL default setup is now not configured. GitHub's Dependency Graph entry is platform-managed (`dynamic/dependabot/update-graph`) and has no repository workflow file; its workflow-disable API request returned HTTP 422.

## Planner refresh

`.github/workflows/planner-refresh.yml` runs weekly on Monday at 12:00 UTC and can also be started manually. Rust is the automation entry point. The Rust crate now handles bounded Green River public-material discovery, annual-schedule link parsing and recent-year filtering, and current-catalog link parsing, catalog pagination and course-heading/body parsing, course credit/description extraction, labeled enrollment/corequisite/satisfies-requirement section parsing, linked enrollment-anchor extraction and context-filtered course-code candidates, enrollment and corequisite AND/OR path parsing with slash shorthand, punctuation/symbol conjunctions, balanced and nested alternatives, grade-context text, and concurrent-enrollment paths. The Node adapter retains a fallback for expressions the Rust parser cannot safely classify. The main degree-requirement parser also delegates batched inline sequence-alternative segmentation and subject-aware carried-course-number expansion for lettered “choose one option” groups to Rust; Node supplies validated explicit-course candidates, constructs the compatibility output, and retains fallback behavior. Rust also handles general-education requirement classification; UW catalog ingestion; official deadline parsing/catalog updates; UW-GRC equivalency-guide row parsing and base rule generation; support-rule resolution; structured/chain derived-rule synthesis; deterministic chunked TypeScript rendering; and equivalency report generation. The Node adapter supplies the local GRC course-title map and declarative derivation specs, then writes Rust-rendered artifacts and reports. The main degree-requirement source parser, Green River catalog fetch/orchestration, and other planner generators still use Node.js/PDF implementations and remain to be migrated; scheduled parser repair is disabled.

The Rust `refresh-planner` command now owns section selection, ordered step execution, cache/target-plan handling, schedule download fallback, source-pipeline repair control flow, source-year coverage reporting, legacy service-test artifact status reporting, and verification orchestration. It invokes the existing standalone CJS parsers and generators where their Rust replacements are not yet production-ready, and calls the Rust catalog, deadline, public-material, source-year coverage, and legacy-artifact components directly. Its workflow entry point continues to default to `--skip-auto-repair`, matching the prior Rust wrapper. The CJS refresh script and `source/package.json` launchers remain unchanged as compatibility references pending full parity review; `--print-step-plan-json` provides a non-refresh plan inspection mode.

For successful scheduled/manual refreshes, the workflow stages only public planner outputs, commits and pushes changed data to `main`, and creates a uniquely tagged GitHub release even when the data is unchanged. It also builds and deploys GitHub Pages. Direct pushes to `main` use the same workflow for Pages deployment without running a planner refresh or creating a release.

## Source fingerprint builder (Rust port in progress)

`build-source-fingerprints` implements the fingerprint/report orchestration and writes the two reports, classification reports, and generated TypeScript registry. Missing prerequisite reports are still produced by the maintained Node scripts. Its current generated-owner index is read from `student-runtime.generated` JSON, while CJS derives that index from TypeScript source-generated models; this is a known parity gap. The existing CJS script and package launchers remain unchanged and authoritative until source-model parity is verified; this Rust command is not a production cutover.

```sh
cargo run --manifest-path automation/Cargo.toml -- \
  build-source-fingerprints [--check-sources-first] [--parse-requirements-first]
```

## Owner-source link probe (Rust CLI)

`probe-owner-source-links` reproduces the owner selection, local-HTML or HTTP fetch, supplemental-document candidate checks, expectations, and console report from `probe-transfer-planner-owner-source-links.cjs`. It loads the canonical TypeScript manifest/bootstrap registries through a small Node bridge; selection and probing use the Rust planner APIs. The CJS reference and package launcher remain unchanged pending broader parity verification.

```sh
cargo run --manifest-path automation/Cargo.toml -- \
  probe-owner-source-links --target-plan-id <plan-id> [checks]
```

## Planner major spot-check (Rust port)

`spot-check-planner-major` reads the generated bootstrap plan array, applies the CJS alias/parent-plan selection rules, and supports listing majors, printing or writing the direct-command report, and selecting targeted refresh/parse/assert/probe actions. The selected operations delegate to their existing CJS scripts, matching the original orchestrator's responsibilities; the CJS reference and package launchers remain unchanged pending parity review. The default command-file path is `source/.tmp/reports/planner-major-spot-check-commands.md`.

```sh
cargo run --manifest-path automation/Cargo.toml -- \
  spot-check-planner-major --list [--campus-id <campus-id>]

cargo run --manifest-path automation/Cargo.toml -- \
  spot-check-planner-major --commands [--campus-id <campus-id>] [--write-command-file [path]]

cargo run --manifest-path automation/Cargo.toml -- \
  spot-check-planner-major --target-plan-id <plan-id> [--standard] [--refresh] [--probe]
```

## Publish-readiness audit (Rust port)

The automation binary exposes `audit-planner-publish-readiness` with `course-source-coverage`, `requirement-source-fidelity`, `student-visible-facts`, and `release-readiness` modes. It writes the selected JSON and Markdown reports under `source/.tmp/reports`; release readiness is the default.

```sh
cargo run --manifest-path automation/Cargo.toml -- \
  audit-planner-publish-readiness [--mode <mode>] [--run-checks]
```

The CJS audit script and its package launchers remain unchanged pending Rust parity verification. `--run-checks` currently retains the CJS parser dependency for parseable-plan enumeration and invokes the existing planner npm checks.

## Source-backed coverage audit (Rust port)

`audit-source-backed-coverage` scores the source-backed coverage, generated-registry, and mapping audits. It reads one planner-context JSON object from stdin and writes the selected JSON and Markdown report under `source/.tmp/reports`.

```sh
cargo run --manifest-path automation/Cargo.toml -- \
  audit-source-backed-coverage [--mapping-only | --generated-registry-only] [--report-only] [--target-plan-id <id>] [--protected-only] --context-stdin
```

`source/scripts/planner/audit-transfer-planner-source-backed-coverage.cjs` remains the `planner:audit:source-backed-coverage` launcher. It still loads the TypeScript planner runtime, quarter plans, and planner audit rows, then asks this command to assemble and write the report. Library exports used by the Node tests stay in that script.

## Monthly Transfer Category Equivalencies refresh

`.github/workflows/transfer-category-equivalencies-monthly.yml` runs on the first day of each month at 12:00 UTC and can also be started manually. It parses the UW Green River Equivalency Guide and regenerates both the planner guide rules and the lightweight Transfer Category Equivalencies catalog. When either public generated file changes, the workflow commits and pushes only those outputs to `main`; the existing Pages workflow handles deployment. The resource copy is checked for “Double check using Equivalency Guide.” and the intentionally blank “Last updated at ____.” placeholder on each run. The broader weekly planner refresh also runs the guide parser as part of its full data refresh.

## Scholarship and internship source review

`.github/workflows/scholarship-refresh.yml` and `internship-refresh.yml` define weekly schedules and manual dispatch, but are deliberately disabled in the repository's Actions settings. They are not run until explicitly enabled. Their source registry is the matching category in `source/data/starter-opportunities.json`; each source is the existing `source.sourceUrl`, falling back to `externalUrl` only when needed.

The Rust collector makes bounded HTTPS requests to those curated URLs, parses page metadata, Schema.org JSON-LD and matching category links deterministically, then uploads a review-only JSON artifact. It does not use AI, crawl discovered links, edit the public catalog, commit, push, or create releases. Candidate facts must be reviewed before they become shared records in the catalog JSON.

Enable a category workflow only when ready:

```sh
gh workflow enable "Scholarship Source Review (Dormant)" --repo MarsLuay/GatorGuide
gh workflow enable "Internship Source Review (Dormant)" --repo MarsLuay/GatorGuide
```

Run the collector locally without changing the catalog:

```sh
cargo run --locked --release --manifest-path automation/Cargo.toml -- \
  collect-opportunities --category scholarships --output /tmp/scholarship-candidates.json

cargo run --locked --release --manifest-path automation/Cargo.toml -- \
  collect-opportunities --category internships --output /tmp/internship-candidates.json
```
